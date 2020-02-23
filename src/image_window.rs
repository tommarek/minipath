use crate::screen_block;

use sdl2;
use image;
use anyhow;
use euclid;
use std::sync;

use image::GenericImage;
use image::GenericImageView;

const SDL_PIXEL_FORMAT: sdl2::pixels::PixelFormatEnum = sdl2::pixels::PixelFormatEnum::ABGR8888;
type PixelType = image::Rgba<u8>;

pub struct ImageWindow {
    context: sdl2::Sdl,
    event: sdl2::EventSubsystem,
    canvas: sdl2::render::Canvas<sdl2::video::Window>,

    // img has to be Arc to avoid issues with double borrowing ImageWindow between
    // run a writer created by make_writer
    img: sync::Arc<sync::Mutex<image::RgbaImage>>,
}

impl ImageWindow {
    /// Creates a SDL window.
    /// There can be only one!
    pub fn new(title: &str, width: u32, height: u32) -> anyhow::Result<ImageWindow> {
        let context = sdl2::init().map_err(anyhow_from_string)?;
        let event = context.event().map_err(anyhow_from_string)?;
        let video = context.video().map_err(anyhow_from_string)?;

        event.register_custom_event::<screen_block::ScreenBlock>().map_err(anyhow_from_string)?;

        let mut canvas = video
            .window(title, width, height)
            .position_centered()
            .resizable()
            .build()?
            .into_canvas()
            .build()?;
        canvas.set_logical_size(width, height)?;

        Ok(ImageWindow {
            context: context,
            event: event,
            canvas: canvas,

            img: sync::Arc::new(sync::Mutex::new(image::ImageBuffer::<PixelType, _>::new(width, height))),
                // This is an Arc to prevent issues with partial borrow
        })
    }

    /// Runs SDL event loop and handles the window.
    /// Only exits when the window is closed.
    pub fn run(&mut self) -> anyhow::Result<()> {
        let (w, h) = self.canvas.logical_size();
        let texture_creator = self.canvas.texture_creator();
        let mut texture = texture_creator.create_texture_streaming(SDL_PIXEL_FORMAT, w, h)?;
        texture.set_blend_mode(sdl2::render::BlendMode::Blend);

        self.update_texture(&mut texture, euclid::size2(w, h).into())?; // Copy the empty output to texture

        let mut events = self.context.event_pump().map_err(anyhow_from_string)?;

        for event in events.wait_iter() {
            use sdl2::event::Event;
            use sdl2::event::WindowEvent;
            use sdl2::keyboard::Keycode;
            match event {
                Event::Quit {..}
                    | Event::KeyDown {keycode: Some(Keycode::Escape), ..}
                    | Event::KeyDown {keycode: Some(Keycode::Q), ..} => break,

                Event::Window {win_event: WindowEvent::Exposed, ..} => self.redraw(&texture).map_err(anyhow_from_string)?,

                _ => if let Some(rendered) = event.as_user_event_type::<screen_block::ScreenBlock>() {
                    self.update_texture(&mut texture, rendered)?;
                    self.redraw(&texture).map_err(anyhow_from_string)?;
                },
            }
        }
        Ok(())
    }

    /// Creates a writer function that can write data into the window from different thread.
    pub fn make_writer(&self) -> impl Fn(screen_block::ScreenBlock, image::RgbaImage) -> anyhow::Result<()> {
        let event_sender = self.event.event_sender();
        let img = self.img.clone();
        move |block: screen_block::ScreenBlock, block_buffer: image::RgbaImage| -> anyhow::Result<()> {
            debug_assert_eq!(block_buffer.width(), block.width());
            debug_assert_eq!(block_buffer.height(), block.width());

            let mut img = (*img).lock().unwrap();
            (*img).copy_from(&block_buffer, block.min.x, block.min.y)?;

            event_sender.push_custom_event(block).map_err(anyhow_from_string)?;

            Ok(())
        }
    }

    /// Copies a block from the image to the texture (to the gpu).
    fn update_texture(&mut self, texture: &mut sdl2::render::Texture, block: screen_block::ScreenBlock) -> anyhow::Result<()> {
        let img = self.img.lock().unwrap();

        let rect = sdl2::rect::Rect::new(block.min.x as i32,
                                         block.min.y as i32,
                                         block.width(),
                                         block.height());

        let (w, _h) = self.canvas.logical_size();
        texture.with_lock(Some(rect), |texture_buffer: &mut [u8], pitch: usize| -> anyhow::Result<()> {
            debug_assert_eq!(pitch, SDL_PIXEL_FORMAT.byte_size_of_pixels(w as usize));

            // Obtain view to the part of the texture that we are updating.
            let mut texture_samples = image::flat::FlatSamples{
                samples: texture_buffer,
                layout: image::flat::SampleLayout{
                    channels: 4, // There is no place to get this value programatically
                    channel_stride: 1, // There is no place to get this value programatically
                    width: block.width(),
                    width_stride: SDL_PIXEL_FORMAT.byte_size_per_pixel(),
                    height: block.height(),
                    height_stride: pitch
                },
                color_hint: None,
            };
            let mut texture_view = texture_samples.as_view_mut::<PixelType>().unwrap();
            texture_view.copy_from(&(*img).view(block.min.x, block.min.y, block.width(), block.height()),
                                   0, 0)?;
            Ok(())
        }).map_err(anyhow_from_string)??;

        Ok(())
    }

    /// Completely redraws the canvas, puts a checkerboard behind and draws the texture on top.
    fn redraw(&mut self, texture: &sdl2::render::Texture) -> Result<(), String> {
        self.draw_checkerboard()?;
        self.canvas.copy(texture, None, None)?;
        self.canvas.present();

        Ok(())
    }

    /// Clears the canvas with a checkerboard pattern.
    fn draw_checkerboard(&mut self) -> Result<(), String> {
        self.canvas.set_draw_color(sdl2::pixels::Color::RGB(50, 50, 50));
        self.canvas.clear();
        self.canvas.set_draw_color(sdl2::pixels::Color::RGB(200, 200, 200));

        let (w, h) = self.canvas.logical_size();
        let checkerboard_size = 20;

        for y in 0..(h / checkerboard_size) {
            for x in ((y % 2)..(w / checkerboard_size)).step_by(2) {
                let rect = sdl2::rect::Rect::new((x * checkerboard_size) as i32,
                                                 (y * checkerboard_size) as i32,
                                                 checkerboard_size,
                                                 checkerboard_size);
                self.canvas.fill_rect(Some(rect))?;
            }
        }

        Ok(())
    }
}

fn anyhow_from_string(e: String) -> anyhow::Error {
    anyhow::anyhow!(e)
}
