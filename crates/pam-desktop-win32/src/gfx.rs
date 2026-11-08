//! Direct2D / Direct3D device resources: the factories, the device shared
//! by every window and the per-window swap chain surface.
//!
//! Everything here lives on the UI thread. The Direct3D device is also
//! handed to Media Foundation (video decoding), which is why it is created
//! with video support and multithread protection.

use pam_desktop_ui::color::Color;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BORDER_MODE_HARD,
    D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_MULTI_THREADED,
    D2D1_GAUSSIANBLUR_PROP_BORDER_MODE, D2D1_LINE_JOIN_ROUND, D2D1_PROPERTY_TYPE_UNKNOWN,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1_UNIT_MODE_DIPS,
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Effect, ID2D1Factory,
    ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MWA_NO_ALT_ENTER, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGISurface, IDXGISwapChain1,
};
use windows::core::{Interface, Result};

use crate::util::color_f;

/// Device-dependent resources. Dropped and rebuilt as a whole when the
/// device is lost.
pub struct Device {
    pub d3d: ID3D11Device,
    pub d3d_context: ID3D11DeviceContext,
    pub dxgi_factory: IDXGIFactory2,
    pub d2d: ID2D1Device,
    /// Draws the windows.
    pub context: ID2D1DeviceContext,
    /// Contexts for offscreen passes (shadows, groups, filters), by nesting depth.
    offscreen: Vec<ID2D1DeviceContext>,
    /// A solid brush recolored before each use.
    pub brush: ID2D1SolidColorBrush,
    /// The blur of `backdropBlur`, made the first time a view needs it.
    backdrop: Option<ID2D1Effect>,
    /// Whether the device can decode video for Media Foundation.
    pub video: bool,
}

pub struct Graphics {
    pub factory: ID2D1Factory1,
    pub dwrite: IDWriteFactory,
    /// Round caps and joins (lines, strokes, dotted borders).
    pub round_stroke: ID2D1StrokeStyle,
    /// Butt caps and round joins for authored vector paths.
    pub path_stroke: ID2D1StrokeStyle,
    pub device: Option<Device>,
    /// Bumped whenever the device is rebuilt: caches keyed by it expire.
    pub generation: u64,
    /// A frame that blurred a backdrop could not be drawn: no more of them.
    pub backdrop_failed: bool,
}

/// The swap chain of one window and the Direct2D bitmap over its back buffer.
pub struct Surface {
    pub swap_chain: IDXGISwapChain1,
    pub target: Option<ID2D1Bitmap1>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    /// The first frame must be presented whole.
    pub presented: bool,
}

fn create_d3d(
    driver: D3D_DRIVER_TYPE,
    flags: D3D11_CREATE_DEVICE_FLAG,
) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let mut device = None;
    let mut context = None;
    // SAFETY: the out pointers refer to live locals; no adapter or software module is passed.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            flags,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(windows::core::Error::from(
            windows::Win32::Foundation::E_FAIL,
        )),
    }
}

impl Device {
    fn new(factory: &ID2D1Factory1) -> Result<Self> {
        // Hardware with video decoding, then hardware, then the software rasterizer.
        let attempts = [
            (
                D3D_DRIVER_TYPE_HARDWARE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                true,
            ),
            (
                D3D_DRIVER_TYPE_HARDWARE,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                false,
            ),
            (
                D3D_DRIVER_TYPE_WARP,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                false,
            ),
        ];
        let mut created = None;
        let mut last_error = windows::core::Error::from(windows::Win32::Foundation::E_FAIL);
        for (driver, flags, video) in attempts {
            match create_d3d(driver, flags) {
                Ok((device, context)) => {
                    created = Some((device, context, video));
                    break;
                }
                Err(error) => last_error = error,
            }
        }
        let Some((d3d, d3d_context, video)) = created else {
            return Err(last_error);
        };
        // Media Foundation decodes on its own threads with this device.
        // The interface is documented on the immediate context; older
        // runtimes expose it on the device.
        if let Ok(multithread) = d3d_context
            .cast::<ID3D11Multithread>()
            .or_else(|_| d3d.cast::<ID3D11Multithread>())
        {
            // SAFETY: plain COM call on a live interface.
            let _ = unsafe { multithread.SetMultithreadProtected(true) };
        }
        let dxgi_device: IDXGIDevice = d3d.cast()?;
        // SAFETY: COM calls on live interfaces created above.
        unsafe {
            let adapter = dxgi_device.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = adapter.GetParent()?;
            let d2d = factory.CreateDevice(&dxgi_device)?;
            let context = d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            configure_context(&context);
            let brush = context.CreateSolidColorBrush(&color_f(Color::BLACK), None)?;
            Ok(Self {
                d3d,
                d3d_context,
                dxgi_factory,
                d2d,
                context,
                offscreen: Vec::new(),
                brush,
                backdrop: None,
                video,
            })
        }
    }

    /// Whether the adapter went away (driver update, remote session, sleep).
    pub fn is_removed(&self) -> bool {
        // SAFETY: plain query on the live device.
        unsafe { self.d3d.GetDeviceRemovedReason() }.is_err()
    }

    /// A context for an offscreen pass at nesting `depth` (0 = the first
    /// pass opened while the window context is drawing).
    pub fn offscreen(&mut self, depth: usize) -> Result<ID2D1DeviceContext> {
        while self.offscreen.len() <= depth {
            // SAFETY: COM call on the live Direct2D device.
            let context = unsafe {
                self.d2d
                    .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?
            };
            configure_context(&context);
            self.offscreen.push(context);
        }
        Ok(self.offscreen[depth].clone())
    }

    /// The Gaussian blur that frosts what is behind a view. Its border is
    /// hard: the rim of the blurred area repeats its edge pixels instead
    /// of fading to nothing.
    pub fn backdrop_blur(&mut self) -> Option<&ID2D1Effect> {
        if self.backdrop.is_none() {
            // SAFETY: COM calls on the live window context; the bytes are
            // the 32-bit value of the enumeration, passed untyped as the
            // C++ `SetValue` helper does.
            unsafe {
                let effect = self.context.CreateEffect(&CLSID_D2D1GaussianBlur).ok()?;
                effect
                    .SetValue(
                        D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32,
                        D2D1_PROPERTY_TYPE_UNKNOWN,
                        &(D2D1_BORDER_MODE_HARD.0 as u32).to_ne_bytes(),
                    )
                    .ok()?;
                self.backdrop = Some(effect);
            }
        }
        self.backdrop.as_ref()
    }

    /// Lets go of the last bitmap the backdrop blur was given, once the
    /// frame that used it is drawn.
    pub fn release_backdrop(&self) {
        if let Some(effect) = &self.backdrop {
            // SAFETY: plain call on the live effect.
            unsafe { effect.SetInput(0, None::<&ID2D1Image>, true) };
        }
    }

    /// The shared solid brush, set to `color`.
    pub fn solid(&self, color: Color) -> &ID2D1SolidColorBrush {
        // SAFETY: the brush belongs to this device and is only used on the UI thread.
        unsafe { self.brush.SetColor(&color_f(color)) };
        &self.brush
    }

    /// A bitmap other contexts can draw into and this device can draw from.
    pub fn create_target_bitmap(&self, width: u32, height: u32, dpi: f32) -> Result<ID2D1Bitmap1> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        // SAFETY: no initial data is passed; the properties outlive the call.
        unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: width.max(1),
                    height: height.max(1),
                },
                None,
                0,
                &properties,
            )
        }
    }

    /// A drawable bitmap from premultiplied BGRA pixels.
    pub fn create_pixel_bitmap(
        &self,
        width: u32,
        height: u32,
        stride: u32,
        pixels: &[u8],
    ) -> Result<ID2D1Bitmap1> {
        if width == 0 || height == 0 || (stride as usize) * (height as usize) > pixels.len() {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: Default::default(),
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        // SAFETY: `pixels` holds at least `stride * height` bytes (checked above).
        unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U { width, height },
                Some(pixels.as_ptr().cast()),
                stride,
                &properties,
            )
        }
    }
}

fn configure_context(context: &ID2D1DeviceContext) {
    // SAFETY: plain setters on a live device context.
    unsafe {
        context.SetUnitMode(D2D1_UNIT_MODE_DIPS);
        // Text is composited over arbitrary content (layers, transforms),
        // where ClearType would fringe.
        context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    }
}

impl Graphics {
    pub fn new() -> Result<Self> {
        // SAFETY: factory creation has no preconditions.
        let factory: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)? };
        // SAFETY: as above.
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let stroke = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        // SAFETY: the properties outlive the call; no dash array is passed.
        let base: &ID2D1Factory = &factory;
        // SAFETY: the properties outlive the call; no dash array is passed.
        let round_stroke = unsafe { base.CreateStrokeStyle(&stroke, None)? };
        let path_properties = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_FLAT,
            endCap: D2D1_CAP_STYLE_FLAT,
            ..stroke
        };
        // SAFETY: the properties outlive the call; no dash array is passed.
        let path_stroke = unsafe { base.CreateStrokeStyle(&path_properties, None)? };
        let device = Device::new(&factory).ok();
        Ok(Self {
            factory: factory.clone(),
            dwrite,
            round_stroke,
            path_stroke,
            device,
            generation: 1,
            backdrop_failed: false,
        })
    }

    /// Makes sure a device exists (after a loss, or a failed start).
    pub fn ensure_device(&mut self) -> bool {
        if self.device.is_none() {
            self.device = Device::new(&self.factory).ok();
            if self.device.is_some() {
                self.generation += 1;
            }
        }
        self.device.is_some()
    }

    /// Drops every device resource; `ensure_device` builds a new device.
    pub fn lose_device(&mut self) {
        self.device = None;
        self.generation += 1;
    }
}

impl Surface {
    pub fn new(graphics: &Graphics, hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let device = graphics
            .device
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        let description = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            // The buffers are resized with the window; never stretch a stale frame.
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        // SAFETY: `hwnd` is a live top-level window of this thread and the description outlives the call.
        let swap_chain = unsafe {
            let swap_chain = device.dxgi_factory.CreateSwapChainForHwnd(
                &device.d3d,
                hwnd,
                &description,
                None,
                None,
            )?;
            // Full screen is a borderless window here, never an exclusive mode switch.
            let _ = device
                .dxgi_factory
                .MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            swap_chain
        };
        Ok(Self {
            swap_chain,
            target: None,
            width: width.max(1),
            height: height.max(1),
            generation: graphics.generation,
            presented: false,
        })
    }

    /// Resizes the back buffers. The target bitmap is rebuilt on the next frame.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return Ok(());
        }
        // Every reference to the back buffer must be gone before resizing.
        self.target = None;
        self.presented = false;
        // SAFETY: the swap chain is live and its buffers are no longer referenced.
        unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?
        };
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// The Direct2D bitmap over the back buffer, at the window's DPI.
    pub fn target(&mut self, device: &Device, dpi: f32) -> Result<ID2D1Bitmap1> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        // SAFETY: buffer 0 of a live swap chain; the properties outlive the call.
        let bitmap = unsafe {
            let surface: IDXGISurface = self.swap_chain.GetBuffer(0)?;
            device
                .context
                .CreateBitmapFromDxgiSurface(&surface, Some(&properties))?
        };
        self.target = Some(bitmap.clone());
        Ok(bitmap)
    }
}
