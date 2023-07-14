use anyhow::Result;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use itertools::iproduct;
use sciimg::prelude::*;
use solhat::ser::{SerFile, SerFrame};

/// Opens a ser file and generates a gtk `Pixbuf`
#[allow(dead_code)]
pub fn picture_from_ser_file(file_path: &str) -> Result<Pixbuf> {
    let ser_file = SerFile::load_ser(file_path).unwrap();
    let first_image = ser_file.get_frame(0).unwrap();
    ser_frame_to_picture(&first_image)
}

/// Converts SerFrame to a gtk `Pixbuf`
pub fn ser_frame_to_picture(ser_frame: &SerFrame) -> Result<Pixbuf> {
    image_to_picture(&ser_frame.buffer)
}

/// Converts `sciimg::Image` to  gtk `Pixbuf`
pub fn image_to_picture(image: &Image) -> Result<Pixbuf> {
    let mut copied = image.clone();
    copied.normalize_to_8bit();

    let pix = Pixbuf::new(
        Colorspace::Rgb,
        false,
        8,
        copied.width as i32,
        copied.height as i32,
    )
    .unwrap();

    iproduct!(0..copied.height, 0..copied.width).for_each(|(y, x)| {
        let (r, g, b) = if copied.num_bands() == 1 {
            (
                copied.get_band(0).get(x, y),
                copied.get_band(0).get(x, y),
                copied.get_band(0).get(x, y),
            )
        } else {
            (
                copied.get_band(0).get(x, y),
                copied.get_band(1).get(x, y),
                copied.get_band(2).get(x, y),
            )
        };
        pix.put_pixel(x as u32, y as u32, r as u8, g as u8, b as u8, 255);
    });
    Ok(pix)
}

/// Converts `sciimg::ImageBuffer` to gtk `Pixbuf`
#[allow(dead_code)]
pub fn imagebuffer_to_picture(buffer: &ImageBuffer) -> Result<Pixbuf> {
    let mut copied = buffer.clone();
    copied.normalize_mut(0.0, 255.0);

    let pix = Pixbuf::new(
        Colorspace::Rgb,
        false,
        8,
        copied.width as i32,
        copied.height as i32,
    )
    .unwrap();

    iproduct!(0..copied.height, 0..copied.width).for_each(|(y, x)| {
        let v = copied.get(x, y);
        pix.put_pixel(x as u32, y as u32, v as u8, v as u8, v as u8, 255);
    });
    Ok(pix)
}
