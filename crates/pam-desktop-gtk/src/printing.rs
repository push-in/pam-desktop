//! Native GTK printer dialog and Cairo/Pango document rendering.
use gtk::{glib::translate::ToGlibPtr, prelude::*};
use pam_desktop_ui::printing::{Document, PrintCompletion};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }
#[link(name = "pangocairo-1.0")]
unsafe extern "C" {
    fn pango_cairo_show_layout(
        cr: *mut gtk::cairo::ffi::cairo_t,
        layout: *mut gtk::pango::ffi::PangoLayout,
    );
}

pub fn print(
    parent: Option<&gtk::ApplicationWindow>,
    document: Document,
    complete: PrintCompletion,
) {
    if ACTIVE.with(|active| active.replace(true)) {
        complete(Err("A print dialog is already open".into()));
        return;
    }
    let callback = Rc::new(RefCell::new(Some(complete)));
    let finish = {
        let callback = Rc::clone(&callback);
        move |result| {
            if let Some(complete) = callback.borrow_mut().take() {
                ACTIVE.with(|active| active.set(false));
                complete(result);
            }
        }
    };
    let operation = gtk::PrintOperation::new();
    operation.set_job_name(&document.title);
    operation.set_n_pages(document.pages.len() as i32);
    operation.set_unit(gtk::Unit::Points);
    operation.set_use_full_page(true);
    operation.set_allow_async(true);
    let setup = gtk::PageSetup::new();
    setup.set_paper_size(&gtk::PaperSize::new_custom(
        "pam-document",
        "PAM Document",
        document.width,
        document.height,
        gtk::Unit::Points,
    ));
    setup.set_top_margin(0.0, gtk::Unit::Points);
    setup.set_bottom_margin(0.0, gtk::Unit::Points);
    setup.set_left_margin(0.0, gtk::Unit::Points);
    setup.set_right_margin(0.0, gtk::Unit::Points);
    operation.set_default_page_setup(Some(&setup));
    operation.connect_draw_page(move |_, context, index| {
        let Some(page) = document.pages.get(index as usize) else {
            return;
        };
        let cairo = context.cairo_context();
        for block in page {
            let layout = context.create_pango_layout();
            let mut font = gtk::pango::FontDescription::from_string("Monospace");
            font.set_absolute_size(block.size * f64::from(gtk::pango::SCALE));
            font.set_weight(if block.bold {
                gtk::pango::Weight::Bold
            } else {
                gtk::pango::Weight::Normal
            });
            layout.set_font_description(Some(&font));
            layout.set_text(&block.text);
            layout.set_width((block.width * f64::from(gtk::pango::SCALE)) as i32);
            layout.set_wrap(gtk::pango::WrapMode::WordChar);
            layout.set_alignment(match block.align {
                2 => gtk::pango::Alignment::Center,
                3 => gtk::pango::Alignment::Right,
                _ => gtk::pango::Alignment::Left,
            });
            if cairo.save().is_err() {
                continue;
            }
            cairo.rectangle(block.x, block.y, block.width, block.height);
            cairo.clip();
            cairo.set_source_rgb(0.0, 0.0, 0.0);
            cairo.move_to(block.x, block.y);
            // SAFETY: the print context and owned Pango layout are live on GTK's UI thread.
            unsafe {
                pango_cairo_show_layout(cairo.to_raw_none(), layout.to_glib_none().0);
            }
            let _ = cairo.restore();
        }
    });
    let done = Rc::clone(&callback);
    operation.connect_done(move |operation, result| {
        if result == gtk::PrintOperationResult::InProgress {
            return;
        }
        if let Some(complete) = done.borrow_mut().take() {
            ACTIVE.with(|active| active.set(false));
            complete(match result {
                gtk::PrintOperationResult::Apply => Ok(true),
                gtk::PrintOperationResult::Cancel => Ok(false),
                _ => Err(operation.error().map_or_else(
                    || "Native printing failed".into(),
                    |error| error.to_string(),
                )),
            });
        }
    });
    match operation.run(gtk::PrintOperationAction::PrintDialog, parent) {
        Ok(gtk::PrintOperationResult::InProgress) => {}
        Ok(gtk::PrintOperationResult::Apply) => finish(Ok(true)),
        Ok(gtk::PrintOperationResult::Cancel) => finish(Ok(false)),
        Ok(_) => finish(Err("Native printing failed".into())),
        Err(error) => finish(Err(error.to_string())),
    }
}
