//! SVG images, which the Windows Imaging Component has no codec for: drawn
//! by Direct2D's SVG renderer (`ID2D1SvgDocument`, Windows 10 1703 and
//! later) into a WIC bitmap on the decoding worker. Like GTK and AppKit, a
//! drawing is rasterized at the size it is shown at (`max_dimension`), or at
//! twice its natural size when that is unknown, so it stays sharp.

use pam_desktop_ui::backend::ImageSource;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1CreateFactory,
    ID2D1DeviceContext5, ID2D1Factory,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapCacheOnLoad,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::Interface;
use windows_numerics::Matrix3x2;

use crate::images::Pixels;

/// The largest side a drawing is rasterized at, in pixels (as GTK).
const VECTOR_LIMIT: u32 = 4096;
/// An SVG file larger than this is refused.
const MAX_SOURCE_BYTES: u64 = 16 << 20;
/// The size of an SVG that states none (CSS replaced element default).
const DEFAULT_SIZE: (f32, f32) = (300.0, 150.0);

/// Whether an image source is an SVG document: a `.svg` file, or bytes
/// whose first element is `<svg`.
pub fn is_svg(source: &ImageSource) -> bool {
    match source {
        ImageSource::File(path) => path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg")),
        ImageSource::Bytes(bytes) => sniff(bytes),
    }
}

/// The first element of the text is `<svg` (after an optional BOM, XML
/// declaration, comments and doctype).
fn sniff(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head)
        .or_else(|error| std::str::from_utf8(&head[..error.valid_up_to()]))
    else {
        return false;
    };
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("<?") {
            let Some(end) = after.find("?>") else {
                return false;
            };
            rest = after[end + 2..].trim_start();
        } else if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else {
                return false;
            };
            rest = after[end + 3..].trim_start();
        } else if let Some(after) = rest.strip_prefix("<!") {
            let Some(end) = after.find('>') else {
                return false;
            };
            rest = after[end + 1..].trim_start();
        } else {
            return rest.starts_with("<svg")
                && rest[4..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/');
        }
    }
}

/// The value of attribute `name` in the start tag `tag`.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut search = tag;
    while let Some(found) = search.find(name) {
        let before = search[..found].chars().last();
        let after = search[found + name.len()..].trim_start();
        search = &search[found + name.len()..];
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let end = value[1..].find(quote)?;
        return Some(&value[1..=end]);
    }
    None
}

/// A length in pixels: unitless or `px` (other units and percentages do
/// not give a natural size).
fn length(value: &str) -> Option<f32> {
    let value = value.trim();
    let number = value.strip_suffix("px").unwrap_or(value).trim();
    number
        .parse::<f32>()
        .ok()
        .filter(|length| length.is_finite() && *length > 0.0)
}

/// The natural size of an SVG document: its `width` and `height`, completed
/// from the aspect ratio of its `viewBox`, or the `viewBox` itself.
pub fn natural_size(bytes: &[u8]) -> (f32, f32) {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)]);
    let Some(start) = text.find("<svg") else {
        return DEFAULT_SIZE;
    };
    let tag = &text[start + 4..];
    let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
    let width = attribute(tag, "width").and_then(length);
    let height = attribute(tag, "height").and_then(length);
    let view_box = attribute(tag, "viewBox").and_then(|value| {
        let numbers: Vec<f32> = value
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse().ok())
            .collect();
        (numbers.len() == 4 && numbers[2] > 0.0 && numbers[3] > 0.0)
            .then(|| (numbers[2], numbers[3]))
    });
    match (width, height, view_box) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some((w, h))) => (width, width * h / w),
        (None, Some(height), Some((w, h))) => (height * w / h, height),
        (None, None, Some(size)) => size,
        (Some(width), None, None) => (width, DEFAULT_SIZE.1),
        (None, Some(height), None) => (DEFAULT_SIZE.0, height),
        (None, None, None) => DEFAULT_SIZE,
    }
}

/// The longest side a drawing is rasterized at: the size it is shown at,
/// or twice its natural size when that is not known (as GTK).
pub fn raster_side(longest: f32, limit: Option<u32>) -> u32 {
    let wanted = match limit.filter(|limit| *limit > 0) {
        Some(limit) => limit,
        None => (longest * 2.0).ceil() as u32,
    };
    wanted.clamp(1, VECTOR_LIMIT)
}

/// The bytes of an SVG source.
pub fn read(source: &ImageSource) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    match source {
        ImageSource::Bytes(bytes) => Ok(std::borrow::Cow::Borrowed(&bytes[..])),
        ImageSource::File(path) => {
            let size = std::fs::metadata(path)
                .map_err(|error| format!("Image file could not be read ({error})"))?
                .len();
            if size > MAX_SOURCE_BYTES {
                return Err("Image is too large".into());
            }
            std::fs::read(path)
                .map(std::borrow::Cow::Owned)
                .map_err(|error| format!("Image file could not be read ({error})"))
        }
    }
}

/// Rasterizes an SVG document to premultiplied BGRA pixels.
pub fn rasterize(
    wic: &IWICImagingFactory,
    bytes: &[u8],
    max_dimension: Option<u32>,
) -> Result<Pixels, String> {
    let failed = |error: windows::core::Error| {
        format!(
            "SVG could not be drawn ({:#010x} {})",
            error.code().0,
            error.message()
        )
    };
    let (natural_width, natural_height) = natural_size(bytes);
    let side = raster_side(natural_width.max(natural_height), max_dimension);
    let ratio = side as f32 / natural_width.max(natural_height);
    let width = ((natural_width * ratio).round() as u32).clamp(1, VECTOR_LIMIT);
    let height = ((natural_height * ratio).round() as u32).clamp(1, VECTOR_LIMIT);
    // SAFETY: COM calls on objects created here; the stream holds its own
    // copy of the bytes; the render target draws into the WIC bitmap, read
    // after EndDraw.
    unsafe {
        let factory: ID2D1Factory =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(failed)?;
        let bitmap = wic
            .CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )
            .map_err(failed)?;
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let target = factory
            .CreateWicBitmapRenderTarget(&bitmap, &properties)
            .map_err(failed)?;
        let context: ID2D1DeviceContext5 = target.cast().map_err(|_| {
            String::from("SVG images need Windows 10 version 1703 or later (Direct2D SVG)")
        })?;
        let stream =
            SHCreateMemStream(Some(bytes)).ok_or_else(|| String::from("SVG could not be read"))?;
        let document = context
            .CreateSvgDocument(
                &stream,
                D2D_SIZE_F {
                    width: natural_width,
                    height: natural_height,
                },
            )
            .map_err(failed)?;
        context.BeginDraw();
        context.Clear(Some(&D2D1_COLOR_F::default()));
        context.SetTransform(&Matrix3x2::scale(
            width as f32 / natural_width,
            height as f32 / natural_height,
        ));
        context.DrawSvgDocument(&document);
        context.EndDraw(None, None).map_err(failed)?;
        let stride = width * 4;
        let mut data = vec![0_u8; stride as usize * height as usize];
        bitmap
            .CopyPixels(std::ptr::null(), stride, &mut data)
            .map_err(failed)?;
        Ok(Pixels {
            width,
            height,
            stride,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn recognizes_svg_documents() {
        let bytes = |text: &str| ImageSource::Bytes(Arc::from(text.as_bytes().to_vec()));
        assert!(is_svg(&bytes("<svg xmlns='http://www.w3.org/2000/svg'/>")));
        assert!(is_svg(&bytes(
            "\u{feff}<?xml version=\"1.0\"?>\n<!-- logo -->\n<!DOCTYPE svg>\n<svg>\n</svg>"
        )));
        assert!(!is_svg(&bytes("<svgx/>")));
        assert!(!is_svg(&bytes("<html><svg/></html>")));
        assert!(!is_svg(&ImageSource::Bytes(Arc::from(
            b"\x89PNG\r\n\x1a\n".to_vec()
        ))));
        assert!(is_svg(&ImageSource::File("logo.SVG".into())));
        assert!(!is_svg(&ImageSource::File("logo.png".into())));
    }

    #[test]
    fn natural_size_follows_width_height_and_view_box() {
        let size = |text: &str| natural_size(text.as_bytes());
        assert_eq!(size("<svg width=\"24\" height=\"12px\"/>"), (24.0, 12.0));
        assert_eq!(size("<svg viewBox=\"0 0 100 50\"/>"), (100.0, 50.0));
        assert_eq!(size("<svg width='40' viewBox='0,0,100,50'/>"), (40.0, 20.0));
        assert_eq!(
            size("<svg height='10' viewBox='0 0 100 50'/>"),
            (20.0, 10.0)
        );
        assert_eq!(
            size("<svg width=\"100%\" viewBox=\"0 0 8 4\"/>"),
            (8.0, 4.0)
        );
        assert_eq!(
            size("<svg stroke-width=\"3\" viewBox=\"0 0 8 4\"/>"),
            (8.0, 4.0)
        );
        assert_eq!(size("<svg/>"), DEFAULT_SIZE);
    }

    #[test]
    fn drawings_take_the_shown_size() {
        assert_eq!(raster_side(100.0, Some(512)), 512);
        assert_eq!(raster_side(100.0, Some(48)), 48);
        assert_eq!(raster_side(100.0, None), 200);
        assert_eq!(raster_side(3000.0, None), VECTOR_LIMIT);
        assert_eq!(raster_side(100.0, Some(0)), 200);
    }

    #[test]
    fn draws_at_the_shown_size_upright() {
        use windows::Win32::Graphics::Imaging::CLSID_WICImagingFactory;
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        };
        // A red top half and a blue bottom half, 20 x 10 natural.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><rect width="20" height="5" fill="#f00"/><rect y="5" width="20" height="5" fill="#00f"/></svg>"##;
        // SAFETY: COM initialization and the in-process WIC factory.
        let wic: IWICImagingFactory = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap()
        };
        match rasterize(&wic, svg, Some(80)) {
            Ok(pixels) => {
                assert_eq!((pixels.width, pixels.height), (80, 40));
                let at = |x: u32, y: u32| {
                    let index = (y * pixels.stride + x * 4) as usize;
                    &pixels.data[index..index + 4]
                };
                // BGRA: red on top, blue at the bottom (not upside down).
                assert_eq!(at(40, 5), [0, 0, 255, 255]);
                assert_eq!(at(40, 35), [255, 0, 0, 255]);
            }
            // Wine's Direct2D has no SVG renderer: there the image fails
            // with a message instead of drawing nothing. Windows must draw.
            Err(message) => assert!(running_on_wine() && message.contains("SVG"), "{message}"),
        }
    }

    fn running_on_wine() -> bool {
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        use windows::core::{s, w};
        // SAFETY: looking up an export of the loaded ntdll.
        unsafe {
            GetModuleHandleW(w!("ntdll.dll"))
                .is_ok_and(|ntdll| GetProcAddress(ntdll, s!("wine_get_version")).is_some())
        }
    }
}
