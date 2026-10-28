//! The small pictures in front of each tree item, drawn at the right size for
//! the monitor.
//!
//! Plain nodes get a folder; every other kind of definition gets a coloured
//! badge with a letter (S for scalar, T for table, and so on). They are drawn
//! with GDI at startup instead of loaded from icon files, so they are sharp at
//! every DPI without shipping a dozen icon sizes.

use windows::Win32::Foundation::{COLORREF, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, FF_DONTCARE, FW_BOLD, FillRect, GdiFlush, HGDIOBJ, OUT_DEFAULT_PRECIS,
    PS_SOLID, Polygon, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    HIMAGELIST, ILC_COLOR32, ImageList_Add, ImageList_Create, ImageList_Destroy,
};
use windows::core::w;

use mib_core::NodeKind;

use fero_win32::dpi;

/// What each image looks like, in image list order.
const GLYPHS: [Glyph; 12] = [
    Glyph::Folder,
    Glyph::Badge("S", 0x2E8B57),        // scalar: green
    Glyph::Badge("T", 0x2F6FBF),        // table: blue
    Glyph::Badge("R", 0x5A4FCF),        // row: indigo
    Glyph::Badge("C", 0x138A8A),        // column: teal
    Glyph::Badge("N", 0xD9822B),        // notification: orange
    Glyph::Badge("G", 0x6B7280),        // group: grey
    Glyph::Badge("\u{2713}", 0x3D7A3D), // compliance: a check mark
    Glyph::Badge("A", 0x8B5E3C),        // agent capabilities: brown
    Glyph::Badge("M", 0x8E44AD),        // module identity: purple
    Glyph::Badge("I", 0x4A6FA5),        // object identity: slate
    Glyph::Badge("?", 0xC0392B),        // unknown: red
];

/// The image list position for a kind of node.
pub fn image_for(kind: NodeKind) -> i32 {
    match kind {
        NodeKind::Internal | NodeKind::Node => 0,
        NodeKind::Scalar => 1,
        NodeKind::Table => 2,
        NodeKind::Row => 3,
        NodeKind::Column => 4,
        NodeKind::Notification => 5,
        NodeKind::Group => 6,
        NodeKind::Compliance => 7,
        NodeKind::Capabilities => 8,
        NodeKind::ModuleIdentity => 9,
        NodeKind::ObjectIdentity => 10,
        NodeKind::Unknown => 11,
    }
}

#[derive(Debug, Clone, Copy)]
enum Glyph {
    Folder,
    /// A letter on a rounded square of this colour (0xRRGGBB).
    Badge(&'static str, u32),
}

/// An image list with one image per kind of node, destroyed when dropped.
#[derive(Debug)]
pub struct NodeIcons {
    list: HIMAGELIST,
}

impl NodeIcons {
    /// Draw the images at the small icon size for `dpi` (16 pixels at 100 %).
    pub fn new(dpi: u32) -> NodeIcons {
        let size = dpi::scale(16, dpi);
        // SAFETY: GDI objects are created, used and deleted within this
        // function, except the image list, which is returned. The DIB
        // section's pixels are only touched after GdiFlush, while it exists.
        unsafe {
            let list = ImageList_Create(size, size, ILC_COLOR32, GLYPHS.len() as i32, 0);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size, // negative: rows from top to bottom
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(None);
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                return NodeIcons { list };
            };
            let old_bitmap = SelectObject(dc, HGDIOBJ(bitmap.0));
            let font = CreateFontW(
                -(size * 3 / 4),
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                FF_DONTCARE.0 as u32,
                w!("Segoe UI"),
            );
            let old_font = SelectObject(dc, HGDIOBJ(font.0));
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);

            for glyph in GLYPHS {
                // Paint the background in a colour no glyph uses, draw, then
                // turn that colour transparent and everything else opaque.
                let background = CreateSolidBrush(colorref(KEY));
                FillRect(
                    dc,
                    &RECT {
                        left: 0,
                        top: 0,
                        right: size,
                        bottom: size,
                    },
                    background,
                );
                let _ = DeleteObject(HGDIOBJ(background.0));
                draw(dc, glyph, size);
                let _ = GdiFlush();
                for pixel in pixels.iter_mut() {
                    *pixel = if *pixel & 0x00FF_FFFF == KEY_BGR {
                        0
                    } else {
                        *pixel | 0xFF00_0000
                    };
                }
                // The image list copies the pixels, so the bitmap is reused.
                SelectObject(dc, old_bitmap);
                ImageList_Add(list, bitmap, None);
                SelectObject(dc, HGDIOBJ(bitmap.0));
            }

            SelectObject(dc, old_font);
            SelectObject(dc, old_bitmap);
            let _ = DeleteObject(HGDIOBJ(font.0));
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
            NodeIcons { list }
        }
    }

    /// The image list, for `TVM_SETIMAGELIST`.
    pub fn handle(&self) -> HIMAGELIST {
        self.list
    }
}

impl Drop for NodeIcons {
    fn drop(&mut self) {
        // SAFETY: we own the image list; the tree has been given a new one
        // (or is gone) before the old one is dropped.
        unsafe {
            let _ = ImageList_Destroy(Some(self.list));
        }
    }
}

/// The background colour that becomes transparent: magenta.
const KEY: u32 = 0xFF00FF;
/// The same colour as stored in a 32-bit DIB (blue, green, red).
const KEY_BGR: u32 = 0x00FF_00FF;

/// GDI wants colours as 0x00BBGGRR.
fn colorref(rgb: u32) -> COLORREF {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    COLORREF(r | (g << 8) | (b << 16))
}

/// A darker shade of a colour, for outlines.
fn darker(rgb: u32) -> u32 {
    let channel = |shift: u32| (((rgb >> shift) & 0xFF) * 3 / 4) << shift;
    channel(16) | channel(8) | channel(0)
}

/// Draw one glyph into a `size` x `size` square.
///
/// # Safety
///
/// `dc` must be a valid memory device context with a bitmap selected.
unsafe fn draw(dc: windows::Win32::Graphics::Gdi::HDC, glyph: Glyph, size: i32) {
    // Coordinates are designed on a 16 x 16 grid and scaled.
    let at = |value: i32| value * size / 16;
    // SAFETY: passed on from the caller; pens and brushes are deleted after use.
    unsafe {
        let (fill, outline) = match glyph {
            Glyph::Folder => (0xE8B84A, 0xB58A2A),
            Glyph::Badge(_, colour) => (colour, darker(colour)),
        };
        let brush = CreateSolidBrush(colorref(fill));
        let pen = CreatePen(PS_SOLID, 1, colorref(outline));
        let old_brush = SelectObject(dc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(dc, HGDIOBJ(pen.0));
        match glyph {
            Glyph::Folder => {
                let points = [(1, 3), (6, 3), (7, 5), (15, 5), (15, 14), (1, 14)]
                    .map(|(x, y)| POINT { x: at(x), y: at(y) });
                let _ = Polygon(dc, &points);
            }
            Glyph::Badge(letter, _) => {
                let _ = RoundRect(dc, at(1), at(1), at(15), at(15), at(5), at(5));
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00FF_FFFF));
                let mut text: Vec<u16> = letter.encode_utf16().collect();
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: size,
                    bottom: size,
                };
                DrawTextW(
                    dc,
                    &mut text,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
            }
        }
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}
