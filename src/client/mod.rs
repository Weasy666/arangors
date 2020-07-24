use std::fmt::Debug;

use http::{HeaderMap, Request, Response};
use url::Url;

use crate::ClientError;

#[cfg(all(feature = "reqwest_async", feature = "reqwest_blocking"))]
compile_error!(
    r#"feature "reqwest_async" and "reqwest_blocking" cannot be set at the same time. 
If what you want is "reqwest_blocking", please turn off default features by adding "default-features=false" in your Cargo.toml"#
);

#[cfg(all(feature = "reqwest_async", feature = "surf_async"))]
compile_error!(
    r#"feature "reqwest_async" and "surf_async" cannot be set at the same time. 
If what you want is "surf_async", please turn off default features by adding "default-features=false" in your Cargo.toml"#
);

#[cfg(all(
    feature = "reqwest_async",
    feature = "reqwest_blocking",
    feature = "surf_async"
))]
compile_error!(
    r#"only one of features "reqwest_async", "reqwest_blocking" and "surf_async" can be"#
);

#[cfg(any(feature = "reqwest_async", feature = "reqwest_blocking"))]
pub mod reqwest;
#[cfg(any(feature = "surf_async"))]
pub mod surf;

#[maybe_async::maybe_async]
pub trait ClientExt: Sync + Debug + Clone {
    fn new<U: Into<Option<HeaderMap>>>(headers: U) -> Result<Self, ClientError>
    where
        Self: Sized;

    #[inline]
    async fn get(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::get(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }
    #[inline]
    async fn post(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::post(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }
    #[inline]
    async fn put(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::put(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }
    #[inline]
    async fn delete(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::delete(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }
    #[inline]
    async fn patch(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::patch(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }

    #[inline]
    async fn connect(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::connect(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }

    #[inline]
    async fn head(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::head(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }

    #[inline]
    async fn options(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::options(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }

    #[inline]
    async fn trace(&self, url: Url, text: &str) -> Result<Response<String>, ClientError>
    where
        Self: Sized,
    {
        self.request(
            Request::trace(url.to_string())
                .body(text.to_string())
                .unwrap(),
        )
        .await
    }

    async fn request(&self, request: Request<String>) -> Result<Response<String>, ClientError>
    where
        Self: Sized;
}
