//! Default copy of the host chrome that has no text from PHP (share menu,
//! alert button, search placeholder fallback, release error screen).
//!
//! English is the base, with Portuguese and Spanish included. The language
//! is the one the app chose with `I18n::use()` (sent as `i18n.locale`),
//! else the process locale. Text that PHP sends (dialog titles, labels)
//! always wins; this catalog only fills the gaps. Every backend uses it, so
//! no backend keeps its own fixed strings.

use std::sync::atomic::{AtomicU8, Ordering};

/// A text of the host chrome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Text {
    Ok,
    Search,
    OpenLink,
    Copy,
    CopyText,
    CopyFile,
    CopyFiles,
    SendByEmail,
    OpenWith,
    ShowInFolder,
    ErrorTitle,
    ErrorMessage,
    TryAgain,
    // Standard application, Edit and Window menus (macOS names; GTK and
    // Win32 use them for role items without a label). `*App` texts hold
    // `{app}`, see [`text_with_app`].
    About,
    AboutApp,
    Hide,
    HideApp,
    HideOthers,
    ShowAll,
    Quit,
    QuitApp,
    Undo,
    Redo,
    Cut,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    EnterFullScreen,
    Close,
    Window,
    Edit,
    Services,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Language {
    English = 1,
    Portuguese = 2,
    Spanish = 3,
}

/// 0: not chosen yet (the process locale is read on first use).
static LANGUAGE: AtomicU8 = AtomicU8::new(0);

fn language_of(tag: &str) -> Language {
    let tag = tag.trim().to_ascii_lowercase();
    let primary = tag.split(['-', '_', '.', '@']).next().unwrap_or("");
    match primary {
        "pt" => Language::Portuguese,
        "es" => Language::Spanish,
        _ => Language::English,
    }
}

/// Follows `tag` (BCP 47 or POSIX, `pt-BR`, `es_ES.UTF-8`); empty goes back
/// to the process locale.
pub fn set_locale(tag: &str) {
    let language = if tag.trim().is_empty() {
        language_of(&crate::modules::system::locale_tag())
    } else {
        language_of(tag)
    };
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}

fn language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        2 => Language::Portuguese,
        3 => Language::Spanish,
        1 => Language::English,
        _ => {
            set_locale("");
            language()
        }
    }
}

/// The text in the chosen language.
#[must_use]
pub fn text(text: Text) -> &'static str {
    translate(language(), text)
}

/// The text with `{app}` replaced by the application name.
#[must_use]
pub fn text_with_app(text: Text, app: &str) -> String {
    self::text(text).replace("{app}", app)
}

/// The default label of a standard menu role, named after `app` when given
/// ("About Name", "Quit Name").
#[must_use]
pub fn menu_role(role: crate::shell::MenuRole, app: Option<&str>) -> String {
    use crate::shell::MenuRole as Role;
    let named = |plain: Text, with_app: Text| match app {
        Some(app) if !app.is_empty() => text_with_app(with_app, app),
        _ => text(plain).to_owned(),
    };
    match role {
        Role::About => named(Text::About, Text::AboutApp),
        Role::Hide => named(Text::Hide, Text::HideApp),
        Role::Quit => named(Text::Quit, Text::QuitApp),
        Role::Preferences => text(Text::Settings).to_owned(),
        Role::Undo => text(Text::Undo).to_owned(),
        Role::Redo => text(Text::Redo).to_owned(),
        Role::Cut => text(Text::Cut).to_owned(),
        Role::Copy => text(Text::Copy).to_owned(),
        Role::Paste => text(Text::Paste).to_owned(),
        Role::SelectAll => text(Text::SelectAll).to_owned(),
        Role::Minimize => text(Text::Minimize).to_owned(),
        Role::Zoom => text(Text::Zoom).to_owned(),
        Role::ToggleFullScreen => text(Text::EnterFullScreen).to_owned(),
        Role::CloseWindow => text(Text::Close).to_owned(),
        Role::HideOthers => text(Text::HideOthers).to_owned(),
        Role::ShowAll => text(Text::ShowAll).to_owned(),
    }
}

/// `label`, or the role's default label when PHP sent none.
#[must_use]
pub fn menu_label<'a>(
    label: &'a str,
    role: Option<crate::shell::MenuRole>,
    app: Option<&str>,
) -> std::borrow::Cow<'a, str> {
    match role {
        Some(role) if label.trim().is_empty() => std::borrow::Cow::Owned(menu_role(role, app)),
        _ => std::borrow::Cow::Borrowed(label),
    }
}

fn translate(language: Language, text: Text) -> &'static str {
    use Language::{English, Portuguese, Spanish};
    use Text::*;
    match (text, language) {
        (Ok, _) => "OK",
        (Search, English) => "Search",
        (Search, Portuguese | Spanish) => "Buscar",
        (OpenLink, English) => "Open link",
        (OpenLink, Portuguese) => "Abrir link",
        (OpenLink, Spanish) => "Abrir enlace",
        (Copy, English) => "Copy",
        (Copy, Portuguese | Spanish) => "Copiar",
        (CopyText, English) => "Copy text",
        (CopyText, Portuguese | Spanish) => "Copiar texto",
        (CopyFile, English) => "Copy file",
        (CopyFile, Portuguese) => "Copiar arquivo",
        (CopyFile, Spanish) => "Copiar archivo",
        (CopyFiles, English) => "Copy files",
        (CopyFiles, Portuguese) => "Copiar arquivos",
        (CopyFiles, Spanish) => "Copiar archivos",
        (SendByEmail, English) => "Send by e-mail…",
        (SendByEmail, Portuguese) => "Enviar por e-mail…",
        (SendByEmail, Spanish) => "Enviar por correo…",
        (OpenWith, English) => "Open with…",
        (OpenWith, Portuguese) => "Abrir com…",
        (OpenWith, Spanish) => "Abrir con…",
        (ShowInFolder, English) => "Show in folder",
        (ShowInFolder, Portuguese) => "Mostrar na pasta",
        (ShowInFolder, Spanish) => "Mostrar en la carpeta",
        (ErrorTitle, English) => "Something went wrong",
        (ErrorTitle, Portuguese) => "Algo deu errado",
        (ErrorTitle, Spanish) => "Algo salió mal",
        (ErrorMessage, English) => "The app hit an unexpected problem. Try again to continue.",
        (ErrorMessage, Portuguese) => {
            "O app encontrou um problema inesperado. Tente novamente para continuar."
        }
        (ErrorMessage, Spanish) => {
            "La aplicación encontró un problema inesperado. Inténtelo de nuevo para continuar."
        }
        (TryAgain, English) => "Try again",
        (TryAgain, Portuguese) => "Tentar novamente",
        (TryAgain, Spanish) => "Intentar de nuevo",
        (About, English) => "About",
        (About, Portuguese) => "Sobre",
        (About, Spanish) => "Acerca de",
        (AboutApp, English) => "About {app}",
        (AboutApp, Portuguese) => "Sobre o {app}",
        (AboutApp, Spanish) => "Acerca de {app}",
        (Hide, English) => "Hide",
        (Hide, Portuguese | Spanish) => "Ocultar",
        (HideApp, English) => "Hide {app}",
        (HideApp, Portuguese | Spanish) => "Ocultar {app}",
        (HideOthers, English) => "Hide Others",
        (HideOthers, Portuguese) => "Ocultar outros",
        (HideOthers, Spanish) => "Ocultar otros",
        (ShowAll, English) => "Show All",
        (ShowAll, Portuguese) => "Mostrar tudo",
        (ShowAll, Spanish) => "Mostrar todo",
        (Quit, English) => "Quit",
        (Quit, Portuguese) => "Sair",
        (Quit, Spanish) => "Salir",
        (QuitApp, English) => "Quit {app}",
        (QuitApp, Portuguese) => "Encerrar {app}",
        (QuitApp, Spanish) => "Salir de {app}",
        (Undo, English) => "Undo",
        (Undo, Portuguese) => "Desfazer",
        (Undo, Spanish) => "Deshacer",
        (Redo, English) => "Redo",
        (Redo, Portuguese) => "Refazer",
        (Redo, Spanish) => "Rehacer",
        (Cut, English) => "Cut",
        (Cut, Portuguese) => "Recortar",
        (Cut, Spanish) => "Cortar",
        (Paste, English) => "Paste",
        (Paste, Portuguese) => "Colar",
        (Paste, Spanish) => "Pegar",
        (SelectAll, English) => "Select All",
        (SelectAll, Portuguese) => "Selecionar tudo",
        (SelectAll, Spanish) => "Seleccionar todo",
        (Minimize, English) => "Minimize",
        (Minimize, Portuguese | Spanish) => "Minimizar",
        (Zoom, _) => "Zoom",
        (EnterFullScreen, English) => "Enter Full Screen",
        (EnterFullScreen, Portuguese) => "Entrar em tela cheia",
        (EnterFullScreen, Spanish) => "Entrar en pantalla completa",
        (Close, English) => "Close",
        (Close, Portuguese) => "Fechar",
        (Close, Spanish) => "Cerrar",
        (Window, English) => "Window",
        (Window, Portuguese) => "Janela",
        (Window, Spanish) => "Ventana",
        (Edit, English) => "Edit",
        (Edit, Portuguese | Spanish) => "Editar",
        (Services, English) => "Services",
        (Services, Portuguese) => "Serviços",
        (Services, Spanish) => "Servicios",
        (Settings, English) => "Settings…",
        (Settings, Portuguese | Spanish) => "Ajustes…",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_follow_the_primary_subtag() {
        assert_eq!(language_of("pt-BR"), Language::Portuguese);
        assert_eq!(language_of("pt_PT.UTF-8"), Language::Portuguese);
        assert_eq!(language_of("es-419"), Language::Spanish);
        assert_eq!(language_of("fr-CA"), Language::English);
        assert_eq!(language_of(""), Language::English);
        assert_eq!(translate(Language::English, Text::Copy), "Copy");
        assert_eq!(
            translate(Language::Portuguese, Text::OpenWith),
            "Abrir com…"
        );
        assert_eq!(
            translate(Language::Spanish, Text::TryAgain),
            "Intentar de nuevo"
        );
        set_locale("es-MX");
        assert_eq!(text(Text::ShowInFolder), "Mostrar en la carpeta");
        set_locale("en-US");
        assert_eq!(text(Text::ShowInFolder), "Show in folder");
        // The standard menus, in the same test: the language is global.
        use crate::shell::MenuRole;
        assert_eq!(
            translate(Language::Portuguese, Text::SelectAll),
            "Selecionar tudo"
        );
        assert_eq!(translate(Language::Spanish, Text::Services), "Servicios");
        set_locale("pt-BR");
        assert_eq!(menu_role(MenuRole::Quit, Some("Caixa")), "Encerrar Caixa");
        assert_eq!(menu_role(MenuRole::Quit, None), "Sair");
        assert_eq!(menu_label("", Some(MenuRole::Paste), None), "Colar");
        assert_eq!(menu_label("Mine", Some(MenuRole::Paste), None), "Mine");
        set_locale("es");
        assert_eq!(menu_role(MenuRole::About, Some("Caja")), "Acerca de Caja");
        set_locale("en");
        assert_eq!(
            menu_role(MenuRole::ToggleFullScreen, None),
            "Enter Full Screen"
        );
    }
}
