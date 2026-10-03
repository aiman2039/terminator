use super::*;
use x11rb::{
    CURRENT_TIME,
    connection::Connection,
    protocol::{
        xproto::{self, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, MapState},
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};
fn x11_coord(value: f64) -> Result<i16> {
    let rounded = value.round();
    ensure!(rounded.is_finite(), "X11 coordinate is not finite: {value}");
    format!("{rounded:.0}")
        .parse()
        .with_context(|| format!("X11 coordinate out of range: {value}"))
}
fn row_stride(width: u16, scanline_pad: u8) -> Result<usize> {
    let pad_bytes = usize::from(scanline_pad)
        .checked_div(8)
        .filter(|pad| *pad > 0)
        .context("X11 scanline pad is zero")?;
    let row = usize::from(width)
        .checked_mul(4)
        .context("X11 row too wide")?;
    row.div_ceil(pad_bytes)
        .checked_mul(pad_bytes)
        .context("X11 stride overflow")
}
fn pixel_index(x: u32, y: u32, stride: usize) -> Result<usize> {
    let row = usize::try_from(y)
        .ok()
        .and_then(|y| y.checked_mul(stride))
        .context("X11 pixel offset")?;
    let column = usize::try_from(x)
        .ok()
        .and_then(|x| x.checked_mul(4))
        .context("X11 pixel offset")?;
    row.checked_add(column).context("X11 pixel offset")
}
fn rgba_image(data: &[u8], width: u16, height: u16, stride: usize) -> Result<image::RgbaImage> {
    let width_px = u32::from(width);
    let height_px = u32::from(height);
    let pixels = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|count| count.checked_mul(4))
        .context("X11 image too large")?;
    let mut buffer = Vec::with_capacity(pixels);
    for y in 0..height_px {
        for x in 0..width_px {
            let index = pixel_index(x, y, stride)?;
            let green = index.checked_add(1).context("X11 pixel offset")?;
            let red = index.checked_add(2).context("X11 pixel offset")?;
            buffer.extend_from_slice(&[
                data.get(red).copied().context("short X11 image")?,
                data.get(green).copied().context("short X11 image")?,
                data.get(index).copied().context("short X11 image")?,
                255,
            ]);
        }
    }
    image::RgbaImage::from_raw(width_px, height_px, buffer).context("X11 image buffer")
}
pub struct Desktop {
    connection: RustConnection,
    root: u32,
    window: u32,
    output: PathBuf,
}
impl Desktop {
    pub fn preflight() -> Result<()> {
        ensure!(
            std::env::var_os("TERMINATOR_X11_TEST").is_some(),
            "Run the X11 fixture in an isolated Xvfb/Openbox display with TERMINATOR_X11_TEST=1"
        );
        Ok(())
    }
    fn atom(&self, name: &str) -> Result<u32> {
        Ok(self
            .connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom)
    }
    pub fn new(pid: u32, output: &Path) -> Result<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .context("X11 screen missing")?
            .root;
        let mut desktop = Self {
            connection,
            root,
            window: 0,
            output: output.into(),
        };
        wait(|| {
            let atom = desktop.atom("_NET_CLIENT_LIST")?;
            let list = desktop
                .connection
                .get_property(false, root, atom, AtomEnum::WINDOW, 0, 256)?
                .reply()?;
            for window in list.value32().into_iter().flatten() {
                let value = desktop
                    .connection
                    .get_property(
                        false,
                        window,
                        desktop.atom("_NET_WM_PID")?,
                        AtomEnum::CARDINAL,
                        0,
                        1,
                    )?
                    .reply()?;
                if value.value32().and_then(|mut v| v.next()) == Some(pid) {
                    desktop.window = window;
                    return Ok(true);
                }
            }
            Ok(false)
        })?;
        Ok(desktop)
    }
    pub fn geometry(&self) -> Result<[f64; 4]> {
        let geometry = self.connection.get_geometry(self.window)?.reply()?;
        let position = self
            .connection
            .translate_coordinates(self.window, self.root, 0, 0)?
            .reply()?;
        Ok([
            f64::from(position.dst_x),
            f64::from(position.dst_y),
            f64::from(geometry.width),
            f64::from(geometry.height),
        ])
    }
    fn event(&self, kind: u8, detail: u8, x: f64, y: f64) -> Result<()> {
        self.connection
            .xtest_fake_input(
                kind,
                detail,
                CURRENT_TIME,
                self.root,
                x11_coord(x)?,
                x11_coord(y)?,
                0,
            )?
            .check()?;
        self.connection.flush()?;
        thread::sleep(Duration::from_millis(45));
        Ok(())
    }
    pub fn click(&mut self, x: f64, y: f64) -> Result<()> {
        self.event(xproto::MOTION_NOTIFY_EVENT, 0, x, y)?;
        self.event(xproto::BUTTON_PRESS_EVENT, 1, x, y)?;
        self.event(xproto::BUTTON_RELEASE_EVENT, 1, x, y)
    }
    pub fn drag(&mut self, start: (f64, f64), end: (f64, f64)) -> Result<()> {
        self.event(xproto::MOTION_NOTIFY_EVENT, 0, start.0, start.1)?;
        self.event(xproto::BUTTON_PRESS_EVENT, 1, start.0, start.1)?;
        for i in 1..=10 {
            self.event(
                xproto::MOTION_NOTIFY_EVENT,
                0,
                start.0 + (end.0 - start.0) * f64::from(i) / 10.0,
                start.1 + (end.1 - start.1) * f64::from(i) / 10.0,
            )?;
        }
        self.event(xproto::BUTTON_RELEASE_EVENT, 1, end.0, end.1)
    }
    pub fn minimize(&mut self) -> Result<()> {
        let r = self.geometry()?;
        self.click(r[0] + 52.0, r[1] + 20.0)
    }
    pub fn minimized(&self) -> Result<bool> {
        Ok(self
            .connection
            .get_window_attributes(self.window)?
            .reply()?
            .map_state
            != MapState::VIEWABLE)
    }
    pub fn restore(&self) -> Result<()> {
        self.connection.map_window(self.window)?.check()?;
        let event = ClientMessageEvent::new(
            32,
            self.window,
            self.atom("_NET_ACTIVE_WINDOW")?,
            [2, CURRENT_TIME, 0, 0, 0],
        );
        self.connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )?
            .check()?;
        self.connection.flush()?;
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        let r = self.geometry()?;
        self.click(r[0] + 20.0, r[1] + 20.0)
    }
    fn dialog(&self) -> Result<Option<u32>> {
        let list = self
            .connection
            .get_property(
                false,
                self.root,
                self.atom("_NET_CLIENT_LIST")?,
                AtomEnum::WINDOW,
                0,
                256,
            )?
            .reply()?;
        for window in list.value32().into_iter().flatten() {
            if window == self.window {
                continue;
            }
            let title = self
                .connection
                .get_property(
                    false,
                    window,
                    self.atom("_NET_WM_NAME")?,
                    AtomEnum::ANY,
                    0,
                    1024,
                )?
                .reply()?;
            let title = String::from_utf8_lossy(&title.value);
            if title == "Open file" || title == "Open project" {
                return Ok(Some(window));
            }
        }
        Ok(None)
    }
    fn capture_dialog(&self) -> Result<()> {
        let Some(window) = self.dialog()? else {
            return Ok(());
        };
        let geometry = self.connection.get_geometry(window)?.reply()?;
        let image = self
            .connection
            .get_image(
                xproto::ImageFormat::Z_PIXMAP,
                window,
                0,
                0,
                geometry.width,
                geometry.height,
                u32::MAX,
            )?
            .reply()?;
        let format = self
            .connection
            .setup()
            .pixmap_formats
            .iter()
            .find(|f| f.depth == image.depth)
            .context("Pixel format missing")?;
        ensure!(
            format.bits_per_pixel == 32,
            "Unsupported X11 capture pixel format"
        );
        let pixels = rgba_image(
            &image.data,
            geometry.width,
            geometry.height,
            row_stride(geometry.width, format.scanline_pad)?,
        )?;
        pixels.save(self.output.join("native-picker.png"))?;
        Ok(())
    }
    fn keycode(&self, symbol: u32) -> Result<(u8, bool)> {
        let setup = self.connection.setup();
        let map = self
            .connection
            .get_keyboard_mapping(
                setup.min_keycode,
                setup
                    .max_keycode
                    .checked_sub(setup.min_keycode)
                    .and_then(|span| span.checked_add(1))
                    .context("X11 keycode range")?,
            )?
            .reply()?;
        let per_keycode = usize::from(map.keysyms_per_keycode);
        ensure!(per_keycode > 0, "X11 keyboard map is empty");
        for (index, symbols) in map.keysyms.chunks(per_keycode).enumerate() {
            for (shift, &code) in symbols.iter().take(2).enumerate() {
                if code == symbol {
                    let keycode = setup
                        .min_keycode
                        .checked_add(u8::try_from(index).context("X11 keycode index")?)
                        .context("X11 keycode overflow")?;
                    return Ok((keycode, shift == 1));
                }
            }
        }
        anyhow::bail!("Key not in X11 keyboard map: {symbol}")
    }
    fn keys(&self, modifiers: &[u32], symbol: u32) -> Result<()> {
        let (code, shift) = self.keycode(symbol)?;
        let mut modifiers = modifiers.to_vec();
        if shift {
            modifiers.push(0xffe1);
        }
        for key in &modifiers {
            self.event(xproto::KEY_PRESS_EVENT, self.keycode(*key)?.0, 0.0, 0.0)?;
        }
        self.event(xproto::KEY_PRESS_EVENT, code, 0.0, 0.0)?;
        self.event(xproto::KEY_RELEASE_EVENT, code, 0.0, 0.0)?;
        for key in modifiers.iter().rev() {
            self.event(xproto::KEY_RELEASE_EVENT, self.keycode(*key)?.0, 0.0, 0.0)?;
        }
        Ok(())
    }
    pub fn open_file_shortcut(&self) -> Result<()> {
        self.keys(&[0xffe3, 0xffe1], u32::from(b'o'))
    }
    pub fn escape(&self) -> Result<()> {
        self.keys(&[], 0xff1b)
    }
    pub fn choose_path(&mut self, path: &str) -> Result<()> {
        if std::path::Path::new(path).is_dir() {
            let window = self.dialog()?.context("Native folder chooser missing")?;
            let position = self
                .connection
                .translate_coordinates(window, self.root, 0, 0)?
                .reply()?;
            let geometry = self.connection.get_geometry(window)?.reply()?;
            // GTK's Recent view has no ordinary filesystem model. Enter Browse
            // through Home before using the location entry to choose a folder.
            self.click(
                f64::from(position.dst_x) + 70.0,
                f64::from(position.dst_y) + 60.0,
            )?;
            thread::sleep(Duration::from_millis(250));
            self.keys(&[0xffe3], u32::from(b'l'))?;
            self.keys(&[0xffe3], u32::from(b'a'))?;
            for byte in format!("{}/", path.trim_end_matches('/')).bytes() {
                self.keys(&[], u32::from(byte))?;
            }
            thread::sleep(Duration::from_millis(200));
            self.keys(&[], 0xff0d)?;
            thread::sleep(Duration::from_millis(500));
            self.capture_dialog()?;
            self.click(
                f64::from(position.dst_x) + f64::from(geometry.width) - 48.0,
                f64::from(position.dst_y) + f64::from(geometry.height) - 22.0,
            )?;
            return Ok(());
        }
        self.keys(&[0xffe3], u32::from(b'l'))?;
        thread::sleep(Duration::from_millis(200));
        for byte in path.bytes() {
            self.keys(&[], u32::from(byte))?;
        }
        self.keys(&[], 0xff0d)?;
        thread::sleep(Duration::from_millis(500));
        self.keys(&[], 0xff0d)
    }
}

#[cfg(test)]
mod tests {
    use super::{rgba_image, row_stride, x11_coord};

    #[test]
    fn stride_rounds_the_row_up_to_the_scanline_pad() {
        assert_eq!(row_stride(1, 32).unwrap(), 4);
        assert_eq!(row_stride(2, 32).unwrap(), 8);
        assert_eq!(row_stride(1, 8).unwrap(), 4);
        assert!(row_stride(1, 0).is_err());
    }

    #[test]
    fn coordinate_stays_inside_the_x11_i16_range() {
        assert_eq!(x11_coord(12.6).unwrap(), 13);
        assert_eq!(x11_coord(-12.6).unwrap(), -13);
        assert_eq!(x11_coord(f64::from(i16::MAX)).unwrap(), i16::MAX);
        assert!(x11_coord(f64::from(i16::MAX) + 1.0).is_err());
        assert!(x11_coord(f64::NAN).is_err());
    }

    #[test]
    fn capture_reads_bgrx_rows_with_padding() {
        let data = [1, 2, 3, 9, 4, 5, 6, 9];
        let image = rgba_image(&data, 1, 2, 4).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [3, 2, 1, 255]);
        assert_eq!(image.get_pixel(0, 1).0, [6, 5, 4, 255]);
        assert!(rgba_image(&[0, 0], 1, 1, 4).is_err());
    }
}
