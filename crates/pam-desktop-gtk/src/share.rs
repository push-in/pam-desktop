//! Sharing on Linux.
//!
//! There is no share sheet to hand text and files to, so the backend shows
//! what a desktop can do with them in a menu at the pointer: copy, send by
//! e-mail, open with another application, show in the file manager.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use pam_desktop_ui::geometry::Point;
use pam_desktop_ui::shell::{MenuNode, ShareRequest};

use crate::shell::Shell;
use crate::view::PamView;

fn item(id: &str, label: &str) -> MenuNode {
    MenuNode::Item {
        id: id.to_owned(),
        label: label.to_owned(),
        accelerator: None,
        enabled: true,
        checked: None,
        role: None,
        destructive: false,
    }
}

/// Text that is nothing but a web address.
fn is_link(text: &str) -> bool {
    let text = text.trim();
    (text.starts_with("https://") || text.starts_with("http://"))
        && !text.contains(char::is_whitespace)
}

/// `xdg-email` attaches files, which a `mailto:` address cannot.
fn mail_tool() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join("xdg-email"))
        .find(|tool| tool.is_file())
}

/// The entries that apply to `request`.
fn entries(request: &ShareRequest, attachments: bool) -> Vec<MenuNode> {
    use pam_desktop_ui::i18n::{Text, text as label};
    let mut menu = Vec::new();
    let text = !request.text.trim().is_empty();
    let files = request.files.len();
    if !text && files == 0 {
        return menu;
    }
    if text && is_link(&request.text) {
        menu.push(item("link", label(Text::OpenLink)));
    }
    if text {
        menu.push(item(
            "copy-text",
            if files > 0 {
                label(Text::CopyText)
            } else {
                label(Text::Copy)
            },
        ));
    }
    if files > 0 {
        menu.push(item(
            "copy-files",
            match (files, text) {
                (1, true) => label(Text::CopyFile),
                (_, true) => label(Text::CopyFiles),
                _ => label(Text::Copy),
            },
        ));
    }
    if files == 0 || attachments {
        menu.push(item("mail", label(Text::SendByEmail)));
    }
    if files > 0 {
        menu.push(MenuNode::Separator);
        if files == 1 {
            menu.push(item("open", label(Text::OpenWith)));
        }
        menu.push(item("folder", label(Text::ShowInFolder)));
    }
    menu
}

/// The pointer inside `host`, or the middle of it when the pointer is
/// elsewhere (the share was started from the keyboard or a timer).
fn anchor(host: &PamView) -> Point {
    let middle = Point {
        x: host.width() as f32 / 2.0,
        y: host.height() as f32 / 2.0,
    };
    let pointer = || {
        let native = host.native()?;
        let surface = native.surface()?;
        let device = host.display().default_seat()?.pointer()?;
        let (x, y, _) = surface.device_position(&device)?;
        let (left, top) = native.surface_transform();
        let point = native.compute_point(
            host,
            &graphene::Point::new((x - left) as f32, (y - top) as f32),
        )?;
        let inside = point.x() >= 0.0
            && point.y() >= 0.0
            && point.x() <= host.width() as f32
            && point.y() <= host.height() as f32;
        inside.then(|| Point {
            x: point.x(),
            y: point.y(),
        })
    };
    pointer().unwrap_or(middle)
}

fn mail(window: &gtk::Window, request: &ShareRequest) {
    if request.files.is_empty() {
        let escape = |text: &str| glib::uri_escape_string(text, None::<&str>, false);
        let uri = format!(
            "mailto:?subject={}&body={}",
            escape(&request.title),
            escape(&request.text)
        );
        gtk::UriLauncher::new(&uri).launch(Some(window), gio::Cancellable::NONE, |_| {});
        return;
    }
    let Some(tool) = mail_tool() else {
        return;
    };
    let mut command = Command::new(tool);
    if !request.title.is_empty() {
        command.arg("--subject").arg(&request.title);
    }
    if !request.text.is_empty() {
        command.arg("--body").arg(&request.text);
    }
    for file in &request.files {
        command.arg("--attach").arg(file);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = command.spawn() {
        // Reaped off the main thread: the tool waits for the mail client to start.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn run(choice: &str, window: &gtk::Window, request: &ShareRequest) {
    let file = |index: usize| request.files.get(index).map(gio::File::for_path);
    match choice {
        "link" => {
            gtk::UriLauncher::new(request.text.trim()).launch(
                Some(window),
                gio::Cancellable::NONE,
                |_| {},
            );
        }
        "copy-text" => window.clipboard().set_text(&request.text),
        "copy-files" => {
            let files: Vec<gio::File> = request.files.iter().map(gio::File::for_path).collect();
            let list = gdk::FileList::from_array(&files);
            let content = gdk::ContentProvider::for_value(&list.to_value());
            let _ = window.clipboard().set_content(Some(&content));
        }
        "mail" => mail(window, request),
        "open" => {
            let launcher = gtk::FileLauncher::new(file(0).as_ref());
            launcher.set_always_ask(true);
            launcher.launch(Some(window), gio::Cancellable::NONE, |_| {});
        }
        "folder" => {
            gtk::FileLauncher::new(file(0).as_ref()).open_containing_folder(
                Some(window),
                gio::Cancellable::NONE,
                |_| {},
            );
        }
        _ => {}
    }
}

/// Opens the share menu over `host`. Answers whether it opened.
pub fn open(
    shell: &Shell,
    window: &gtk::ApplicationWindow,
    host: &PamView,
    request: &ShareRequest,
) -> bool {
    let menu = entries(request, mail_tool().is_some());
    if menu.is_empty() || !window.is_mapped() {
        return false;
    }
    let (window, request) = (window.downgrade(), request.clone());
    shell.popup_menu(
        host,
        anchor(host),
        &menu,
        Box::new(move |choice| {
            if let (Some(choice), Some(window)) = (choice, window.upgrade()) {
                run(&choice, window.upcast_ref(), &request);
            }
        }),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(request: &ShareRequest, attachments: bool) -> Vec<String> {
        entries(request, attachments)
            .into_iter()
            .map(|node| match node {
                MenuNode::Item { id, .. } => id,
                _ => "-".to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_menu_follows_what_is_shared() {
        let text = ShareRequest {
            text: "hello".into(),
            ..ShareRequest::default()
        };
        assert_eq!(ids(&text, false), ["copy-text", "mail"]);

        let link = ShareRequest {
            text: " https://example.com/a?b=1 ".into(),
            ..ShareRequest::default()
        };
        assert_eq!(ids(&link, false), ["link", "copy-text", "mail"]);
        assert!(!is_link("see https://example.com"));

        let file = ShareRequest {
            files: vec!["/tmp/report.pdf".into()],
            ..ShareRequest::default()
        };
        // Without a tool that attaches files there is no e-mail entry.
        assert_eq!(ids(&file, false), ["copy-files", "-", "open", "folder"]);
        assert_eq!(
            ids(&file, true),
            ["copy-files", "mail", "-", "open", "folder"]
        );

        let both = ShareRequest {
            text: "Two reports".into(),
            files: vec!["/tmp/a.pdf".into(), "/tmp/b.pdf".into()],
            ..ShareRequest::default()
        };
        assert_eq!(
            ids(&both, true),
            ["copy-text", "copy-files", "mail", "-", "folder"]
        );
        assert!(entries(&ShareRequest::default(), true).is_empty());
    }
}
