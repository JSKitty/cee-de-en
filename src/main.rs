use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::net::SocketAddr;
use std::convert::Infallible;
use std::sync::Arc;

use hyper::{Server, Request, Response, Body, Method, StatusCode};
use hyper::service::{service_fn, make_service_fn};

use minify_html;
use minify_js;

use brotlic::{BlockSize, BrotliEncoderOptions, CompressorWriter, Quality, WindowSize};

// Utility function for serving content via it's byte form
async fn serve_content(
    req: Request<Body>,
    content: Arc<Vec<u8>>,
) -> Result<Response<Body>, Infallible> {
    match req.method() {
        // Serve the content for every GET request
        &Method::GET => Ok(
            Response::builder()
                .status(200)
                .header("content-encoding", "br")
                .header("content-type", "text/javascript") // TODO: automate the content-type filling
                .body(hyper::Body::from((*content).clone())).unwrap()
        ),

        // All other routes are 404s
        _ => Ok(
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body("No such resource".into())
                .unwrap()
        ),
    }
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // TODO: read settings from a config file, Serde ftw
    // Note: maybe we want to accept some basic CLI input too?

    // TODO: read address and port from config
    let addr = String::from("0.0.0.0:1337").parse::<SocketAddr>().unwrap();

    // Build the Hyper server
    let svc_builder = make_service_fn(move |_conn| {
        async {
            // Create our 'CDN' endpoint which essentially just assumes the request path to be a *relative* disk path
            Ok::<_, Infallible>(
                service_fn(move |req: Request<Body>| {
                    println!("Serving resource: {}", req.uri());

                    // Parse the resource path (chopping off the initial `/`)
                    let path_string = &req.uri().path().to_string()[1..];
                    let path = PathBuf::from(&path_string);

                    // TODO: Parse the file type into "content-type" suitable strings, we can then use them to
                    // ... select the correct minifier, decide to compress or not, and also for responding with the correct Content-Type

                    // Read from resource path (TODO: better handling of missing files)
                    let file_contents = fs::read(path).unwrap_or(format!("Naw the file {path_string} don't exist here m8").into_bytes());

                    // First: we minify anything code-wise
                    // TODO: only minify [js, html, css]
                    let mut minify_cfg = minify_html::Cfg::new();
                    minify_cfg.keep_comments = false;

                    let minified = minify_html::minify(&file_contents, &minify_cfg);

                    // Second: we apply Brotli compression
                    // TODO: move the encoder options outside of the service scope (pre-load) and load config values set by the user, if set.
                    let encoder = BrotliEncoderOptions::new()
                        .quality(Quality::best())
                        .window_size(WindowSize::best())
                        .block_size(BlockSize::best())
                        .build().unwrap();

                    let compressor_storage = Vec::new();
                    let mut compressed_writer = CompressorWriter::with_encoder(encoder, compressor_storage);

                    // TODO: catch any weird compression errors and fallback to raw file (why would these happen?)
                    compressed_writer.write_all(minified.as_slice()).unwrap();
                    let compressed_file = compressed_writer.into_inner().unwrap();

                    // For science: log the benefits we achieved from the above steps!
                    println!("Resource was [{}] bytes, reduced to [{}] via minifying and then [{}] by Brotli", file_contents.len(), minified.len(), compressed_file.len());
                    
                    // Capture the file contents in an Arc so we can use the reference repeatedly
                    // across async tasks that the server will spawn
                    let resource_finalised = Arc::new(compressed_file);

                    // Serve the compressed bytes!
                    serve_content(req, resource_finalised)
                })
            )
        }
    });

    // Start up our service and accept connections
    println!("Starting server...");
    let server = Server::bind(&addr).serve(svc_builder);
    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }

    Ok(())
}
