//! Pictures from the history, which may be encrypted on disk.

use std::sync::Arc;

use gpui_kit::*;

use crate::store::Blob;

/// Reads and decrypts a stored picture in the background, once.
enum Stored {}

impl Asset for Stored {
    type Source = Blob;
    type Output = Option<Arc<Image>>;

    // An `async fn` would borrow `App`, which the future must outlive.
    #[allow(clippy::manual_async_fn)]
    fn load(blob: Blob, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let format = if blob.file_name().ends_with(".svg") {
                ImageFormat::Svg
            } else {
                ImageFormat::Png
            };
            blob.read()
                .map(|bytes| Arc::new(Image::from_bytes(format, bytes)))
                .inspect_err(|error| {
                    tracing::warn!(error = format!("{error:#}"), "could not load a picture");
                })
                .ok()
        }
    }
}

/// Like `gpui::img`, for a stored picture.
pub fn img(blob: Blob) -> Img {
    gpui_kit::img(ImageSource::Custom(Arc::new(move |window, cx| {
        let image = window.use_asset::<Stored>(&blob, cx)??;
        image.use_render_image(window, cx).map(Ok)
    })))
}
