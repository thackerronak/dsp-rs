#[cfg(feature = "tck")]
pub(super) mod tck {
    use axum::{
        body::Body,
        extract::Request,
        http::{Response, StatusCode},
        middleware::Next,
        response::IntoResponse,
    };
    use futures::StreamExt;
    use http_body_util::BodyExt;
    use tracing::{debug, error};

    pub(crate) async fn log_req_resp(req: Request, next: Next) -> impl IntoResponse {
        let is_websocket = req
            .headers()
            .get("upgrade")
            .map(|v| v.to_str().unwrap_or("") == "websocket")
            .unwrap_or(false);

        if is_websocket {
            return next.run(req).await;
        }

        let (parts, body) = req.into_parts();
        let bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to collect request body: {}", e);
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("could not read request body"))
                    .expect("builder is safe with valid status/body");
            }
        };
        if let Ok(body_str) = std::str::from_utf8(&bytes) {
            debug!("<-- request = {}", body_str);
        } else {
            debug!("<-- request = <non-utf8 {} bytes>", bytes.len());
        }

        let response = next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
        let (parts, body) = response.into_parts();

        let res_stream = body.into_data_stream().map(|chunk| match chunk {
            Ok(bytes) => {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    debug!("--> response chunk = {}", s);
                } else {
                    debug!("--> response chunk = <non-utf8 {} bytes>", bytes.len());
                }
                Ok(bytes)
            }
            Err(e) => Err(e),
        });

        Response::from_parts(parts, Body::from_stream(res_stream))
    }
}
