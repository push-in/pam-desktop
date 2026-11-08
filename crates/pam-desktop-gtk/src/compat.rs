//! GTK APIs newer than the GTK 4.6 baseline.
//!
//! The backend is built against GTK 4.6 (Ubuntu 22.04, Debian 12 ships
//! 4.8), so one binary runs on every supported distribution. Each newer
//! function is looked up in the loaded `libgtk-4` the first time it is
//! needed; when the running GTK has it, it is used, otherwise the caller
//! takes a GTK 4.6 path.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;

use gtk::glib::translate::{FromGlibPtrFull, IntoGlib, ToGlibPtr, ToGlibPtrMut, from_glib};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk};
use pam_desktop_ui::backend::ImageFit;

type Object = *mut c_void;
type Ready = Option<unsafe extern "C" fn(Object, Object, *mut c_void)>;
type Error = *mut *mut glib::ffi::GError;

fn lookup(name: &CStr) -> usize {
    // SAFETY: a lookup in the global scope, where libgtk-4 is loaded.
    unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) as usize }
}

/// Declares accessors returning the function when the running GTK has it.
macro_rules! newer {
    ($(fn $name:ident($($arg:ty),* $(,)?) $(-> $ret:ty)?;)*) => {$(
        #[allow(non_snake_case)]
        fn $name() -> Option<unsafe extern "C" fn($($arg),*) $(-> $ret)?> {
            static ADDRESS: OnceLock<usize> = OnceLock::new();
            let address = *ADDRESS.get_or_init(|| {
                lookup(&CString::new(stringify!($name)).unwrap_or_default())
            });
            // SAFETY: the symbol is the GTK function of that name, whose C
            // signature is the one declared here.
            (address != 0).then(|| unsafe {
                std::mem::transmute::<usize, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(address)
            })
        }
    )*};
}

newer! {
    // GTK 4.8
    fn gtk_event_controller_scroll_get_unit(Object) -> c_int;
    fn gtk_picture_set_content_fit(Object, c_int);
    fn gdk_file_list_new_from_array(*mut *mut gio::ffi::GFile, usize) -> *mut gdk::ffi::GdkFileList;
    // GTK 4.10
    fn gtk_widget_get_color(Object, *mut gdk::ffi::GdkRGBA);
    fn gtk_snapshot_push_mask(Object, c_int);
    fn gtk_snapshot_append_scaled_texture(Object, Object, c_int, *const graphene::ffi::graphene_rect_t);
    fn gtk_alert_dialog_get_type() -> glib::ffi::GType;
    fn gtk_alert_dialog_choose(Object, Object, Object, Ready, *mut c_void);
    fn gtk_alert_dialog_choose_finish(Object, Object, Error) -> c_int;
    fn gtk_file_dialog_get_type() -> glib::ffi::GType;
    fn gtk_file_dialog_open(Object, Object, Object, Ready, *mut c_void);
    fn gtk_file_dialog_open_finish(Object, Object, Error) -> Object;
    fn gtk_file_dialog_open_multiple(Object, Object, Object, Ready, *mut c_void);
    fn gtk_file_dialog_open_multiple_finish(Object, Object, Error) -> Object;
    fn gtk_file_dialog_save(Object, Object, Object, Ready, *mut c_void);
    fn gtk_file_dialog_save_finish(Object, Object, Error) -> Object;
    fn gtk_file_dialog_select_folder(Object, Object, Object, Ready, *mut c_void);
    fn gtk_file_dialog_select_folder_finish(Object, Object, Error) -> Object;
    fn gtk_uri_launcher_get_type() -> glib::ffi::GType;
    fn gtk_uri_launcher_launch(Object, Object, Object, Ready, *mut c_void);
    fn gtk_file_launcher_get_type() -> glib::ffi::GType;
    fn gtk_file_launcher_launch(Object, Object, Object, Ready, *mut c_void);
    fn gtk_file_launcher_open_containing_folder(Object, Object, Object, Ready, *mut c_void);
    // GTK 4.12
    fn gtk_css_provider_load_from_string(Object, *const c_char);
    // GTK 4.14
    fn gtk_accessible_announce(Object, *const c_char, c_int);
    fn gsk_path_builder_new() -> Object;
    fn gsk_path_builder_add_rounded_rect(Object, *const gsk::ffi::GskRoundedRect);
    fn gsk_path_builder_free_to_path(Object) -> Object;
    fn gsk_path_unref(Object);
    fn gsk_stroke_new(f32) -> Object;
    fn gsk_stroke_set_line_cap(Object, c_int);
    fn gsk_stroke_set_dash(Object, *const f32, usize);
    fn gsk_stroke_free(Object);
    fn gtk_snapshot_append_stroke(Object, Object, Object, *const gdk::ffi::GdkRGBA);
}

fn raw<T: ObjectType>(object: &T) -> Object {
    object.as_ptr().cast()
}

fn window(parent: Option<&gtk::Window>) -> Object {
    parent.map_or(ptr::null_mut(), raw)
}

/// An instance of a GTK type known by its `get_type` function.
fn instance(get_type: Option<unsafe extern "C" fn() -> glib::ffi::GType>) -> Option<glib::Object> {
    // SAFETY: a GTK `get_type` function returns a registered type.
    let type_: glib::Type = unsafe { from_glib(get_type?()) };
    Some(glib::Object::with_type(type_))
}

/// The C callback of an asynchronous GTK call, calling `done` with the
/// result.
fn ready(done: impl FnOnce(Object) + 'static) -> (Ready, *mut c_void) {
    type Done = Box<dyn FnOnce(Object)>;
    unsafe extern "C" fn trampoline(_source: Object, result: Object, data: *mut c_void) {
        // SAFETY: `data` is the box leaked below, called back once.
        let done = unsafe { Box::from_raw(data.cast::<Done>()) };
        done(result);
    }
    let done: Box<Done> = Box::new(Box::new(done));
    (Some(trampoline), Box::into_raw(done).cast())
}

/// Calls the `_finish` function of an asynchronous call returning an
/// object, dropping the error.
fn finish<T>(
    finish: unsafe extern "C" fn(Object, Object, Error) -> Object,
    source: &glib::Object,
    result: Object,
) -> Option<T>
where
    T: ObjectType + FromGlibPtrFull<*mut T::GlibType>,
{
    let mut error = ptr::null_mut();
    // SAFETY: `result` is the result of the call on `source`.
    let object = unsafe { finish(raw(source), result, &mut error) };
    if !error.is_null() {
        // SAFETY: an error the caller owns.
        drop(unsafe { glib::Error::from_glib_full(error) });
    }
    // SAFETY: the `_finish` functions return a full reference.
    (!object.is_null()).then(|| unsafe { T::from_glib_full(object.cast()) })
}

/// Whether a scroll controller reports wheel notches. GTK before 4.8
/// always reports scrolling in wheel steps.
pub fn scrolls_in_wheel_steps(controller: &gtk::EventControllerScroll) -> bool {
    gtk_event_controller_scroll_get_unit().is_none_or(|unit| {
        // SAFETY: a scroll controller.
        unsafe { unit(raw(controller)) == gdk::ffi::GDK_SCROLL_UNIT_WHEEL }
    })
}

/// How a picture fills its box. GTK before 4.8 can only keep the aspect
/// ratio (contain) or stretch (fill).
pub fn set_content_fit(picture: &gtk::Picture, fit: ImageFit) {
    let Some(set) = gtk_picture_set_content_fit() else {
        picture.set_keep_aspect_ratio(fit != ImageFit::Fill);
        return;
    };
    let fit = match fit {
        ImageFit::Contain => gtk::ffi::GTK_CONTENT_FIT_CONTAIN,
        ImageFit::Fill => gtk::ffi::GTK_CONTENT_FIT_FILL,
        ImageFit::Center | ImageFit::Repeat => gtk::ffi::GTK_CONTENT_FIT_SCALE_DOWN,
        ImageFit::Cover => gtk::ffi::GTK_CONTENT_FIT_COVER,
    };
    // SAFETY: a picture and a `GtkContentFit`.
    unsafe { set(raw(picture), fit) }
}

/// Clipboard content holding `paths` as files: a `GdkFileList` from GTK
/// 4.8, a `text/uri-list` before.
pub fn file_list(paths: &[impl AsRef<Path>]) -> gdk::ContentProvider {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    if let Some(new) = gdk_file_list_new_from_array() {
        let mut pointers: Vec<*mut gio::ffi::GFile> =
            files.iter().map(|file| file.as_ptr()).collect();
        // SAFETY: an array of files, copied by GDK; the list is returned full.
        let list =
            unsafe { gdk::FileList::from_glib_full(new(pointers.as_mut_ptr(), pointers.len())) };
        return gdk::ContentProvider::for_value(&list.to_value());
    }
    let list: String = files
        .iter()
        .map(|file| format!("{}\r\n", file.uri()))
        .collect();
    gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(list.into_bytes()))
}

/// The foreground color of `widget`; from its style context before GTK 4.10.
pub fn widget_color(widget: &impl IsA<gtk::Widget>) -> gdk::RGBA {
    let Some(get) = gtk_widget_get_color() else {
        return widget.as_ref().style_context().color();
    };
    let mut color = gdk::RGBA::BLACK;
    // SAFETY: a widget and a color to fill.
    unsafe { get(raw(widget.as_ref()), color.to_glib_none_mut().0) };
    color
}

/// Pushes an alpha mask: the first child up to the next `pop` is the mask
/// of the second. Answers false on GTK before 4.10, which has none.
pub fn push_alpha_mask(snapshot: &gtk::Snapshot) -> bool {
    let Some(push) = gtk_snapshot_push_mask() else {
        return false;
    };
    // SAFETY: a snapshot and a `GskMaskMode`.
    unsafe { push(raw(snapshot), gsk::ffi::GSK_MASK_MODE_ALPHA) };
    true
}

/// A texture drawn into `bounds` with `filter`; GTK before 4.10 draws it
/// with its default (linear) filter.
pub fn append_scaled_texture(
    snapshot: &gtk::Snapshot,
    texture: &gdk::Texture,
    filter: gsk::ScalingFilter,
    bounds: &graphene::Rect,
) {
    match gtk_snapshot_append_scaled_texture() {
        // SAFETY: a snapshot, a texture, a `GskScalingFilter` and a rectangle.
        Some(append) => unsafe {
            append(
                raw(snapshot),
                raw(texture),
                filter.into_glib(),
                bounds.to_glib_none().0,
            );
        },
        None => snapshot.append_texture(texture, bounds),
    }
}

/// Strokes the outline of `rect` with dashes `dash`, round caps when
/// `round`. Answers false on GTK before 4.14, which strokes no paths.
pub fn stroke_rounded_rect(
    snapshot: &gtk::Snapshot,
    rect: &gsk::RoundedRect,
    width: f32,
    dash: &[f32],
    round: bool,
    color: &gdk::RGBA,
) -> bool {
    let (
        Some(builder_new),
        Some(add_rounded_rect),
        Some(to_path),
        Some(path_unref),
        Some(stroke_new),
        Some(set_line_cap),
        Some(set_dash),
        Some(stroke_free),
        Some(append_stroke),
    ) = (
        gsk_path_builder_new(),
        gsk_path_builder_add_rounded_rect(),
        gsk_path_builder_free_to_path(),
        gsk_path_unref(),
        gsk_stroke_new(),
        gsk_stroke_set_line_cap(),
        gsk_stroke_set_dash(),
        gsk_stroke_free(),
        gtk_snapshot_append_stroke(),
    )
    else {
        return false;
    };
    // SAFETY: the GSK 4.14 path and stroke API, each object freed once.
    unsafe {
        let builder = builder_new();
        add_rounded_rect(builder, rect.to_glib_none().0);
        let path = to_path(builder);
        let stroke = stroke_new(width);
        if round {
            set_line_cap(stroke, gsk::ffi::GSK_LINE_CAP_ROUND);
        }
        set_dash(stroke, dash.as_ptr(), dash.len());
        append_stroke(raw(snapshot), path, stroke, color.to_glib_none().0);
        stroke_free(stroke);
        path_unref(path);
    }
    true
}

/// Loads a style sheet.
pub fn load_css(provider: &gtk::CssProvider, css: &str) {
    match (gtk_css_provider_load_from_string(), CString::new(css)) {
        // SAFETY: a provider and a NUL-terminated string.
        (Some(load), Ok(css)) => unsafe { load(raw(provider), css.as_ptr()) },
        _ => provider.load_from_data(css),
    }
}

/// Asks the screen reader to speak `message`. Answers false on GTK before
/// 4.14, which has no announcements.
pub fn announce(widget: &impl IsA<gtk::Widget>, message: &str, assertive: bool) -> bool {
    let (Some(announce), Ok(message)) = (gtk_accessible_announce(), CString::new(message)) else {
        return false;
    };
    let priority = if assertive {
        gtk::ffi::GTK_ACCESSIBLE_ANNOUNCEMENT_PRIORITY_HIGH
    } else {
        gtk::ffi::GTK_ACCESSIBLE_ANNOUNCEMENT_PRIORITY_MEDIUM
    };
    // SAFETY: a widget is an accessible.
    unsafe { announce(raw(widget.as_ref()), message.as_ptr(), priority) };
    true
}

/// Opens `uri` with the application that handles it.
pub fn launch_uri(parent: &gtk::Window, uri: &str) {
    let Some((launcher, launch)) =
        instance(gtk_uri_launcher_get_type()).zip(gtk_uri_launcher_launch())
    else {
        gtk::show_uri(Some(parent), uri, gdk::CURRENT_TIME);
        return;
    };
    launcher.set_property("uri", uri);
    // SAFETY: a launcher and a window; without a callback the result is dropped.
    unsafe {
        launch(
            raw(&launcher),
            raw(parent),
            ptr::null_mut(),
            None,
            ptr::null_mut(),
        )
    };
}

/// Asks which application should open `file`, then opens it.
pub fn open_with(parent: &gtk::Window, file: &gio::File) {
    // Asking first needs GTK 4.12 (`always-ask`).
    let launcher = instance(gtk_file_launcher_get_type())
        .filter(|launcher| launcher.find_property("always-ask").is_some());
    if let Some((launcher, launch)) = launcher.zip(gtk_file_launcher_launch()) {
        launcher.set_property("file", file);
        launcher.set_property("always-ask", true);
        // SAFETY: a launcher and a window; without a callback the result is dropped.
        unsafe {
            launch(
                raw(&launcher),
                raw(parent),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            )
        };
        return;
    }
    let dialog = gtk::AppChooserDialog::new(
        Some(parent),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        file,
    );
    let file = file.clone();
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Ok
            && let Some(application) = dialog.app_info()
        {
            let context = WidgetExt::display(dialog).app_launch_context();
            let _ = application.launch(std::slice::from_ref(&file), Some(&context));
        }
        dialog.destroy();
    });
    dialog.present();
}

/// Shows `file` selected in the file manager.
pub fn show_in_folder(parent: &gtk::Window, file: &gio::File) {
    if let Some((launcher, open)) =
        instance(gtk_file_launcher_get_type()).zip(gtk_file_launcher_open_containing_folder())
    {
        launcher.set_property("file", file);
        // SAFETY: a launcher and a window; without a callback the result is dropped.
        unsafe {
            open(
                raw(&launcher),
                raw(parent),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            )
        };
        return;
    }
    // What GTK 4.10 does: the file manager interface, else the folder.
    let folder = file.parent().map(|folder| folder.uri());
    let parent = parent.downgrade();
    gio::bus_get(gio::BusType::Session, gio::Cancellable::NONE, {
        let uri = file.uri();
        move |connection| {
            let fallback = move || {
                if let (Some(parent), Some(folder)) = (parent.upgrade(), folder) {
                    gtk::show_uri(Some(&parent), &folder, gdk::CURRENT_TIME);
                }
            };
            let Ok(connection) = connection else {
                return fallback();
            };
            connection.call(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&(vec![uri.as_str()], "").to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
                move |reply| {
                    if reply.is_err() {
                        fallback();
                    }
                },
            );
        }
    });
}

/// A message with buttons. `done` gets the index of the chosen button, the
/// cancel button when dismissed, or `None` when dismissed without one.
pub struct Alert<'a> {
    pub message: &'a str,
    pub detail: &'a str,
    pub buttons: Vec<&'a str>,
    pub cancel: Option<usize>,
    pub default: usize,
}

pub fn choose(parent: Option<&gtk::Window>, alert: &Alert, done: Box<dyn FnOnce(Option<usize>)>) {
    let index = |index: usize| i32::try_from(index).unwrap_or(-1);
    if let Some((dialog, choose)) =
        instance(gtk_alert_dialog_get_type()).zip(gtk_alert_dialog_choose())
    {
        dialog.set_property("modal", true);
        dialog.set_property("message", alert.message);
        dialog.set_property("detail", alert.detail);
        dialog.set_property("buttons", glib::StrV::from(alert.buttons.clone()));
        dialog.set_property("cancel-button", alert.cancel.map_or(-1, index));
        dialog.set_property("default-button", index(alert.default));
        let source = dialog.clone();
        let (callback, data) = ready(move |result| {
            let chosen = gtk_alert_dialog_choose_finish().and_then(|finish| {
                let mut error = ptr::null_mut();
                // SAFETY: the result of `choose` on `source`.
                let chosen = unsafe { finish(raw(&source), result, &mut error) };
                if !error.is_null() {
                    // SAFETY: an error the caller owns.
                    drop(unsafe { glib::Error::from_glib_full(error) });
                }
                usize::try_from(chosen).ok()
            });
            done(chosen);
        });
        // SAFETY: a dialog, an optional window and the callback above.
        unsafe {
            choose(
                raw(&dialog),
                window(parent),
                ptr::null_mut(),
                callback,
                data,
            )
        };
        return;
    }
    let dialog = gtk::MessageDialog::builder()
        .modal(true)
        .text(alert.message)
        .secondary_text(alert.detail)
        .buttons(gtk::ButtonsType::None)
        .build();
    dialog.set_transient_for(parent);
    for (position, label) in alert.buttons.iter().enumerate() {
        let id = u16::try_from(position).unwrap_or(u16::MAX);
        dialog.add_button(label, gtk::ResponseType::Other(id));
    }
    dialog.set_default_response(gtk::ResponseType::Other(
        u16::try_from(alert.default).unwrap_or(0),
    ));
    let cancel = alert.cancel;
    let done = std::cell::Cell::new(Some(done));
    dialog.connect_response(move |dialog, response| {
        let chosen = match response {
            gtk::ResponseType::Other(id) => Some(usize::from(id)),
            _ => cancel,
        };
        if let Some(done) = done.take() {
            done(chosen);
        }
        dialog.destroy();
    });
    dialog.present();
}

/// What a file dialog asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Open,
    OpenMultiple,
    Save,
    Folder,
}

pub struct FileDialog<'a> {
    pub pick: Pick,
    pub title: &'a str,
    pub initial_name: &'a str,
    pub initial_folder: Option<gio::File>,
    pub filters: Vec<gtk::FileFilter>,
}

pub fn pick_files(
    parent: Option<&gtk::Window>,
    config: FileDialog,
    done: Box<dyn FnOnce(Vec<gio::File>)>,
) {
    let calls = match config.pick {
        Pick::Open => gtk_file_dialog_open().zip(gtk_file_dialog_open_finish()),
        Pick::OpenMultiple => {
            gtk_file_dialog_open_multiple().zip(gtk_file_dialog_open_multiple_finish())
        }
        Pick::Save => gtk_file_dialog_save().zip(gtk_file_dialog_save_finish()),
        Pick::Folder => gtk_file_dialog_select_folder().zip(gtk_file_dialog_select_folder_finish()),
    };
    if let Some((dialog, (start, end))) = instance(gtk_file_dialog_get_type()).zip(calls) {
        dialog.set_property("modal", true);
        if !config.title.is_empty() {
            dialog.set_property("title", config.title);
        }
        if !config.initial_name.is_empty() {
            dialog.set_property("initial-name", config.initial_name);
        }
        if let Some(folder) = &config.initial_folder {
            dialog.set_property("initial-folder", folder);
        }
        if !config.filters.is_empty() {
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.extend_from_slice(&config.filters);
            dialog.set_property("filters", &filters);
        }
        let source = dialog.clone();
        let multiple = config.pick == Pick::OpenMultiple;
        let (callback, data) = ready(move |result| {
            let files = if multiple {
                finish::<gio::ListModel>(end, &source, result)
                    .map(|files| files.iter::<gio::File>().filter_map(Result::ok).collect())
                    .unwrap_or_default()
            } else {
                finish::<gio::File>(end, &source, result)
                    .into_iter()
                    .collect()
            };
            done(files);
        });
        // SAFETY: a dialog, an optional window and the callback above.
        unsafe {
            start(
                raw(&dialog),
                window(parent),
                ptr::null_mut(),
                callback,
                data,
            )
        };
        return;
    }
    let action = match config.pick {
        Pick::Save => gtk::FileChooserAction::Save,
        Pick::Folder => gtk::FileChooserAction::SelectFolder,
        Pick::Open | Pick::OpenMultiple => gtk::FileChooserAction::Open,
    };
    let dialog = gtk::FileChooserNative::new(Some(config.title), parent, action, None, None);
    dialog.set_modal(true);
    dialog.set_select_multiple(config.pick == Pick::OpenMultiple);
    if !config.initial_name.is_empty() && config.pick == Pick::Save {
        dialog.set_current_name(config.initial_name);
    }
    if let Some(folder) = &config.initial_folder {
        let _ = dialog.set_current_folder(Some(folder));
    }
    for filter in &config.filters {
        dialog.add_filter(filter);
    }
    // The dialog lives until it answers.
    let state = std::cell::RefCell::new(Some((dialog.clone(), done)));
    dialog.connect_response(move |dialog, response| {
        let Some((_, done)) = state.borrow_mut().take() else {
            return;
        };
        let files = if response == gtk::ResponseType::Accept {
            dialog
                .files()
                .iter::<gio::File>()
                .filter_map(Result::ok)
                .collect()
        } else {
            Vec::new()
        };
        done(files);
    });
    dialog.show();
}
