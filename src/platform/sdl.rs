//! SDL3 backend: one window, one streaming RGBA texture of 320x200, presented with an
//! integer scale factor (nearest neighbour) and letterboxing.

use super::{Backend, Event, Key, Mods, MouseButton, PlatformOptions};
use crate::assets::Palette;
use crate::gfx::{Surface, SCREEN_H, SCREEN_W};
use sdl3::event::{Event as SdlEvent, WindowEvent};
use sdl3::keyboard::{Keycode, Mod, Scancode};
use sdl3::pixels::PixelFormat;
use sdl3::rect::Rect as SdlRect;
use sdl3::render::{ScaleMode, Texture, TextureCreator, WindowCanvas};
use sdl3::video::WindowContext;
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream, AudioStreamWithCallback};
use sdl3::EventPump;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Audio-thread side of the sound engine: renders whatever SDL asks for.
struct SoundCallback {
    engine: Arc<Mutex<crate::audio::Engine>>,
    buf: Vec<i16>,
}

impl AudioCallback<i16> for SoundCallback {
    fn callback(&mut self, stream: &mut AudioStream, requested: i32) {
        let n = (requested.max(0) as usize + 1) & !1;
        self.buf.clear();
        self.buf.resize(n, 0);
        if let Ok(mut e) = self.engine.lock() {
            e.render(&mut self.buf);
        }
        let _ = stream.put_data_i16(&self.buf);
    }
}

pub struct SdlBackend {
    _sdl: sdl3::Sdl,
    canvas: WindowCanvas,
    _creator: TextureCreator<WindowContext>,
    texture: Texture,
    pump: EventPump,
    rgba: Vec<u8>,
    start: Instant,
    next_frame: Instant,
    scale: u32,
    offset: (i32, i32),
    fullscreen: bool,
    text_input: sdl3::keyboard::TextInputUtil,
    audio: Option<AudioStreamWithCallback<SoundCallback>>,
}

const FRAME: Duration = Duration::from_nanos(16_666_667);

impl SdlBackend {
    pub fn init(o: &PlatformOptions) -> Result<SdlBackend, String> {
        let sdl = sdl3::init().map_err(|e| e.to_string())?;
        let video = sdl.video().map_err(|e| e.to_string())?;
        let scale = match o.scale {
            Some(s) => s.max(1),
            None => {
                let (w, h) = video
                    .get_primary_display()
                    .and_then(|d| d.get_usable_bounds())
                    .map(|r| (r.width(), r.height()))
                    .unwrap_or((1280, 800));
                super::pick_scale(w, h)
            }
        };
        let mut builder = video.window(&o.title, SCREEN_W as u32 * scale, SCREEN_H as u32 * scale);
        builder.position_centered().resizable();
        if o.fullscreen {
            builder.fullscreen();
        }
        let window = builder.build().map_err(|e| e.to_string())?;
        let text_input = video.text_input();
        text_input.start(&window);
        let canvas = window.into_canvas();
        let creator = canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormat::ABGR8888, SCREEN_W as u32, SCREEN_H as u32)
            .map_err(|e| e.to_string())?;
        texture.set_scale_mode(ScaleMode::Nearest);
        let pump = sdl.event_pump().map_err(|e| e.to_string())?;
        sdl.mouse().show_cursor(false);
        let now = Instant::now();
        let mut b = SdlBackend {
            _sdl: sdl,
            canvas,
            _creator: creator,
            texture,
            pump,
            rgba: vec![0; (SCREEN_W * SCREEN_H * 4) as usize],
            start: now,
            next_frame: now + FRAME,
            scale,
            offset: (0, 0),
            fullscreen: o.fullscreen,
            text_input,
            audio: None,
        };
        b.recompute_layout();
        Ok(b)
    }

    fn recompute_layout(&mut self) {
        let (w, h) = self.canvas.output_size().unwrap_or((SCREEN_W as u32, SCREEN_H as u32));
        let k = ((w / SCREEN_W as u32).min(h / SCREEN_H as u32)).max(1);
        self.scale = k;
        let ox = (w as i32 - SCREEN_W * k as i32) / 2;
        let oy = (h as i32 - SCREEN_H * k as i32) / 2;
        self.offset = (ox.max(0), oy.max(0));
    }

    fn to_screen(&self, x: f32, y: f32) -> (i32, i32) {
        let sx = (x as i32 - self.offset.0).div_euclid(self.scale as i32);
        let sy = (y as i32 - self.offset.1).div_euclid(self.scale as i32);
        (sx, sy)
    }

    /// Window position of the centre of screen pixel (x, y); the inverse of `to_screen`.
    fn to_window(&self, x: i32, y: i32) -> (f32, f32) {
        let s = self.scale as i32;
        ((self.offset.0 + x * s + s / 2) as f32, (self.offset.1 + y * s + s / 2) as f32)
    }

    fn mods(m: Mod) -> Mods {
        Mods {
            shift: m.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
            ctrl: m.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
            alt: m.intersects(Mod::LALTMOD | Mod::RALTMOD),
        }
    }

    fn translate_key(keycode: Option<Keycode>, scancode: Option<Scancode>) -> Key {
        // Keypad first (independent of NumLock).
        if let Some(sc) = scancode {
            let kp = match sc {
                Scancode::Kp0 => Some(0),
                Scancode::Kp1 => Some(1),
                Scancode::Kp2 => Some(2),
                Scancode::Kp3 => Some(3),
                Scancode::Kp4 => Some(4),
                Scancode::Kp5 => Some(5),
                Scancode::Kp6 => Some(6),
                Scancode::Kp7 => Some(7),
                Scancode::Kp8 => Some(8),
                Scancode::Kp9 => Some(9),
                _ => None,
            };
            if let Some(n) = kp {
                return Key::Kp(n);
            }
            match sc {
                Scancode::KpEnter => return Key::Enter,
                Scancode::KpPlus => return Key::Plus,
                Scancode::KpMinus => return Key::Minus,
                _ => {}
            }
        }
        let Some(kc) = keycode else { return Key::Unknown };
        match kc {
            Keycode::Up => Key::Up,
            Keycode::Down => Key::Down,
            Keycode::Left => Key::Left,
            Keycode::Right => Key::Right,
            Keycode::Home => Key::Home,
            Keycode::End => Key::End,
            Keycode::PageUp => Key::PageUp,
            Keycode::PageDown => Key::PageDown,
            Keycode::Return | Keycode::Return2 | Keycode::KpEnter => Key::Enter,
            Keycode::Escape => Key::Esc,
            Keycode::Space => Key::Space,
            Keycode::Backspace => Key::Backspace,
            Keycode::Tab => Key::Tab,
            Keycode::Delete => Key::Delete,
            Keycode::Insert => Key::Insert,
            Keycode::F1 => Key::F(1),
            Keycode::F2 => Key::F(2),
            Keycode::F3 => Key::F(3),
            Keycode::F4 => Key::F(4),
            Keycode::F5 => Key::F(5),
            Keycode::F6 => Key::F(6),
            Keycode::F7 => Key::F(7),
            Keycode::F8 => Key::F(8),
            Keycode::F9 => Key::F(9),
            Keycode::F10 => Key::F(10),
            Keycode::F11 => Key::F(11),
            Keycode::F12 => Key::F(12),
            Keycode::Plus | Keycode::KpPlus | Keycode::Equals => Key::Plus,
            Keycode::Minus | Keycode::KpMinus => Key::Minus,
            other => {
                let v = other.to_ll().0 as i32;
                if (0x20..0x7F).contains(&v) {
                    Key::Char((v as u8 as char).to_ascii_lowercase())
                } else {
                    Key::Unknown
                }
            }
        }
    }

    pub fn window_title(&mut self, title: &str) {
        let _ = self.canvas.window_mut().set_title(title);
    }
}

impl Backend for SdlBackend {
    fn warp_mouse(&mut self, x: i32, y: i32) {
        let (wx, wy) = self.to_window(x, y);
        self._sdl.mouse().warp_mouse_in_window(self.canvas.window(), wx, wy);
    }

    fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        let events: Vec<SdlEvent> = self.pump.poll_iter().collect();
        for ev in events {
            match ev {
                SdlEvent::Quit { .. } => out.push(Event::Quit),
                SdlEvent::Window { win_event, .. } => match win_event {
                    WindowEvent::Resized(_, _) | WindowEvent::PixelSizeChanged(_, _) => self.recompute_layout(),
                    _ => {}
                },
                SdlEvent::KeyDown { keycode, scancode, keymod, repeat, .. } => {
                    let key = Self::translate_key(keycode, scancode);
                    let mods = Self::mods(keymod);
                    // Alt+Enter toggles fullscreen.
                    if key == Key::Enter && mods.alt && !repeat {
                        let fs = !self.fullscreen;
                        self.set_fullscreen(fs);
                        continue;
                    }
                    out.push(Event::Key { key, mods, down: true, repeat });
                }
                SdlEvent::KeyUp { keycode, scancode, keymod, .. } => {
                    let key = Self::translate_key(keycode, scancode);
                    out.push(Event::Key { key, mods: Self::mods(keymod), down: false, repeat: false });
                }
                SdlEvent::TextInput { text, .. } => {
                    for c in text.chars() {
                        out.push(Event::Text(c));
                    }
                }
                SdlEvent::MouseMotion { x, y, .. } => {
                    let (sx, sy) = self.to_screen(x, y);
                    out.push(Event::MouseMove { x: sx, y: sy });
                }
                SdlEvent::MouseButtonDown { mouse_btn, x, y, .. } | SdlEvent::MouseButtonUp { mouse_btn, x, y, .. } => {
                    let down = matches!(ev, SdlEvent::MouseButtonDown { .. });
                    let button = match mouse_btn {
                        sdl3::mouse::MouseButton::Left => MouseButton::Left,
                        sdl3::mouse::MouseButton::Right => MouseButton::Right,
                        sdl3::mouse::MouseButton::Middle => MouseButton::Middle,
                        _ => continue,
                    };
                    let (sx, sy) = self.to_screen(x, y);
                    out.push(Event::MouseButton { button, x: sx, y: sy, down });
                }
                _ => {}
            }
        }
        out
    }

    fn present(&mut self, fb: &Surface, pal: &Palette) {
        fb.to_rgba(pal, &mut self.rgba);
        let _ = self.texture.update(None, &self.rgba, (SCREEN_W * 4) as usize);
        self.canvas.set_draw_color((0, 0, 0, 255));
        self.canvas.clear();
        let k = self.scale as i32;
        let dst = SdlRect::new(self.offset.0, self.offset.1, (SCREEN_W * k) as u32, (SCREEN_H * k) as u32);
        let _ = self.canvas.copy(&self.texture, None, Some(dst.into()));
        self.canvas.present();
    }

    fn wait_frame(&mut self) {
        let now = Instant::now();
        if self.next_frame > now {
            std::thread::sleep(self.next_frame - now);
        }
        let now = Instant::now();
        self.next_frame += FRAME;
        if self.next_frame < now {
            // Fell behind: resynchronise.
            self.next_frame = now + FRAME;
        }
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn scale(&self) -> u32 {
        self.scale
    }

    fn set_fullscreen(&mut self, on: bool) {
        if self.canvas.window_mut().set_fullscreen(on).is_ok() {
            self.fullscreen = on;
            self.recompute_layout();
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }
    fn start_audio(&mut self, engine: Arc<Mutex<crate::audio::Engine>>) -> Result<(), String> {
        self.open_audio(engine)
    }
}

impl SdlBackend {
    fn open_audio(&mut self, engine: Arc<Mutex<crate::audio::Engine>>) -> Result<(), String> {
        let audio = self._sdl.audio().map_err(|e| e.to_string())?;
        let rate = engine.lock().map_err(|_| "sound engine poisoned".to_string())?.rate() as i32;
        let spec = AudioSpec { freq: Some(rate), channels: Some(2), format: Some(AudioFormat::S16LE) };
        let stream = audio
            .open_playback_stream(&spec, SoundCallback { engine, buf: Vec::new() })
            .map_err(|e| e.to_string())?;
        stream.resume().map_err(|e| e.to_string())?;
        self.audio = Some(stream);
        Ok(())
    }
}

impl Drop for SdlBackend {
    fn drop(&mut self) {
        self.text_input.stop(self.canvas.window());
    }
}
