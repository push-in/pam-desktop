//! AppKit printer/PDF panel backed by a native paginated text view.
use crate::text::Paragraph;
use crate::util::ns_string;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSGraphicsContext, NSPrintInfo, NSPrintOperation, NSView};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSRange, NSRect};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::printing::{Document, PrintCompletion};
use pam_desktop_ui::text::{FontSpec, TextAlign, TextBlock, TextRun};
use std::cell::Cell;

thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Document]
    struct PrintView;
    unsafe impl NSObjectProtocol for PrintView {}
    impl PrintView {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }

        #[unsafe(method(knowsPageRange:))]
        unsafe fn page_range(&self, range: *mut NSRange) -> bool {
            if range.is_null() { false } else {
                // SAFETY: AppKit supplies a writable NSRange for this callback.
                unsafe { *range = NSRange::new(1, self.ivars().pages.len()); }
                true
            }
        }
        #[unsafe(method(rectForPage:))]
        fn page_rect(&self, page: isize) -> NSRect {
            let document = self.ivars();
            CGRect::new(CGPoint::new(0.0, (page.max(1) - 1) as f64 * document.height), CGSize::new(document.width, document.height))
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, dirty: NSRect) {
            let Some(graphics) = NSGraphicsContext::currentContext() else { return; };
            let Some(context) = graphics_context(&graphics) else { return; };
            let document = self.ivars();
            let first = (dirty.origin.y / document.height).floor().max(0.0) as usize;
            let last = ((dirty.origin.y + dirty.size.height) / document.height).ceil().max(0.0) as usize;
            for index in first..last.min(document.pages.len()) {
                for block in &document.pages[index] {
                    let paragraph = Paragraph::new(&TextBlock {
                        runs: vec![TextRun { text: block.text.clone(), font: FontSpec { family: "Menlo".into(), size: block.size as f32, weight: if block.bold { 700 } else { 400 }, ..Default::default() }, color: Some(Color::BLACK), background: None, underline: false, strikethrough: false, letter_spacing: 0.0, press: None }],
                        align: match block.align { 2 => TextAlign::Center, 3 => TextAlign::End, _ => TextAlign::Start },
                        ..Default::default()
                    });
                    let y = block.y + index as f64 * document.height;
                    CGContext::save_g_state(Some(&context));
                    CGContext::clip_to_rect(Some(&context), CGRect::new(CGPoint::new(block.x, y), CGSize::new(block.width, block.height)));
                    paragraph.draw(&context, CGPoint::new(block.x, y), block.width as f32, None);
                    CGContext::restore_g_state(Some(&context));
                }
            }
        }
    }
);

/// The Core Graphics context of a drawing context. The print preview draws
/// through `NSPrintPreviewGraphicsContext`, an `NSProxy` that forwards
/// `CGContext` without declaring it, so the message is sent unchecked (a
/// checked send panics, and a panic inside `drawRect:` aborts).
fn graphics_context(graphics: &NSGraphicsContext) -> Option<CFRetained<CGContext>> {
    // SAFETY: every `NSGraphicsContext`, proxied or not, answers `CGContext`
    // with a context it owns (or NULL); it is retained before use.
    unsafe {
        let send: unsafe extern "C-unwind" fn(*const NSGraphicsContext, Sel) -> *mut CGContext =
            std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
        let context = std::ptr::NonNull::new(send(graphics, sel!(CGContext)))?;
        Some(CFRetained::retain(context))
    }
}

pub fn print(document: Document, complete: PrintCompletion) {
    let Some(mtm) = MainThreadMarker::new() else {
        complete(Err("Printing requires AppKit's main thread".into()));
        return;
    };
    if ACTIVE.with(|active| active.replace(true)) {
        complete(Err("A print dialog is already open".into()));
        return;
    }
    let info = NSPrintInfo::new();
    info.setPaperSize(CGSize::new(document.width, document.height));
    info.setLeftMargin(0.0);
    info.setRightMargin(0.0);
    info.setTopMargin(0.0);
    info.setBottomMargin(0.0);
    info.setHorizontallyCentered(false);
    info.setVerticallyCentered(false);
    let title = ns_string(&document.title);
    let frame = CGRect::new(
        CGPoint::ZERO,
        CGSize::new(
            document.width,
            document.height * document.pages.len() as f64,
        ),
    );
    let allocated = PrintView::alloc(mtm).set_ivars(document);
    // SAFETY: designated NSView initializer and retained native print operation.
    let view: Retained<PrintView> = unsafe { msg_send![super(allocated), initWithFrame: frame] };
    let operation = NSPrintOperation::printOperationWithView_printInfo(&view, &info);
    operation.setJobTitle(Some(&title));
    operation.setShowsPrintPanel(true);
    operation.setShowsProgressPanel(true);
    let accepted = operation.runOperation();
    ACTIVE.with(|active| active.set(false));
    complete(Ok(accepted));
}
