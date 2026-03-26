use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Body, Incoming},
    header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE},
    service::Service,
};

const KEEP_ALIVE_FRAME: &[u8] = b": keep-alive\n\n";

pub fn is_initial_sse_probe(request: &Request<Incoming>) -> bool {
    request.method() == Method::GET
        && accepts_event_stream(request)
        && !has_non_empty_session_id(request)
}

pub async fn handle_request<S, B>(
    service: S,
    request: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, B::Error>>, S::Error>
where
    S: Service<Request<Incoming>, Response = Response<B>> + Clone,
    B: Body<Data = Bytes> + Send + Sync + 'static,
    B::Error: Send + Sync + 'static,
{
    if is_initial_sse_probe(&request) {
        return Ok(initial_sse_probe_response());
    }

    let response = Service::call(&service, request).await?;
    let response = rewrite_unauthorized_status(response);

    Ok(response.map(|body| body.boxed()))
}

fn accepts_event_stream(request: &Request<Incoming>) -> bool {
    request
        .headers()
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| {
            accept
                .split(',')
                .map(str::trim)
                .any(|value| value.eq_ignore_ascii_case("text/event-stream"))
        })
}

fn has_non_empty_session_id(request: &Request<Incoming>) -> bool {
    request
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !value.trim().is_empty())
}

fn initial_sse_probe_response<E>() -> Response<BoxBody<Bytes, E>>
where
    E: Send + Sync + 'static,
{
    let body = Full::new(Bytes::from_static(KEEP_ALIVE_FRAME))
        .map_err(|never| match never {})
        .boxed();

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header(CACHE_CONTROL, "no-cache")
        .body(body)
        .expect("compat SSE probe response should be valid")
}

fn rewrite_unauthorized_status<B>(response: Response<B>) -> Response<B> {
    if response.status() != StatusCode::UNAUTHORIZED {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    parts.status = StatusCode::BAD_REQUEST;
    Response::from_parts(parts, body)
}
