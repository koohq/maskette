//! Ported from the old backend's `tests/test_references.py`, plus the path
//! checks of the asset route.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::{DIR_NAME, MAX_UPLOAD_BYTES, safe_filename};
use crate::backend::tests::{TOKEN, TestBackend, authed, json_body};
use crate::backend::token;

const BOUNDARY: &str = "maskette-test-boundary";

enum Part<'a> {
    Text(&'a str, &'a str),
    File {
        name: &'a str,
        file_name: &'a str,
        content_type: Option<&'a str>,
        bytes: &'a [u8],
    },
}

fn multipart(parts: &[Part]) -> Vec<u8> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match part {
            Part::Text(name, value) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                        .as_bytes(),
                );
            }
            Part::File {
                name,
                file_name,
                content_type,
                bytes,
            } => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n"
                    )
                    .as_bytes(),
                );
                if let Some(content_type) = content_type {
                    body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
                }
                body.extend_from_slice(b"\r\n");
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
        }
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn upload(app: &Router, parts: &[Part<'_>]) -> Response {
    let request = authed(Method::POST, "/api/references")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart(parts)))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

fn image<'a>(file_name: &'a str, content_type: &'a str, bytes: &'a [u8]) -> Part<'a> {
    Part::File {
        name: "file",
        file_name,
        content_type: Some(content_type),
        bytes,
    }
}

async fn get(app: &Router, uri: &str) -> Response {
    app.clone()
        .oneshot(authed(Method::GET, uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn bytes(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

async fn assert_detail(response: Response, status: StatusCode, detail: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(json_body(response).await["detail"], detail);
}

#[tokio::test]
async fn reference_upload_stores_image_and_returns_object_key() {
    let backend = TestBackend::new(true);
    let app = backend.app();

    let response = upload(
        &app,
        &[
            Part::Text("kind", "front"),
            image("front.webp", "image/webp", b"webp-image"),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload = json_body(response).await;
    assert_eq!(payload["file_name"], "front.webp");
    let key = payload["object_key"].as_str().unwrap();
    assert!(key.starts_with("references/"), "{key}");
    assert!(key.ends_with("/front.webp"), "{key}");
    let id = key.split('/').nth(1).unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().nth(14), Some('4'));

    let stored = backend
        .dir
        .path()
        .join(DIR_NAME)
        .join(key.strip_prefix("references/").unwrap());
    assert_eq!(std::fs::read(stored).unwrap(), b"webp-image");

    // Served back at the URL the other routers hand out, with either token.
    let response = get(&app, &format!("/api/references/{key}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
    assert_eq!(bytes(response).await, b"webp-image");
    let uri = format!("/api/references/{key}?{}={TOKEN}", token::QUERY_PARAM);
    let response = app
        .clone()
        .oneshot(axum::http::Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["cross-origin-resource-policy"],
        "cross-origin"
    );
}

#[tokio::test]
async fn reference_upload_rejects_non_image_files() {
    let app = TestBackend::new(true).app();

    let response = upload(
        &app,
        &[
            Part::Text("kind", "front"),
            image("prompt.txt", "text/plain", b"not image"),
        ],
    )
    .await;
    assert_detail(response, StatusCode::BAD_REQUEST, "unsupported_image_type").await;

    // An image name with a type that is not an image is refused too.
    let response = upload(
        &app,
        &[
            Part::Text("kind", "front"),
            image("front.png", "text/plain", b"not image"),
        ],
    )
    .await;
    assert_detail(response, StatusCode::BAD_REQUEST, "unsupported_image_type").await;
}

#[tokio::test]
async fn reference_upload_accepts_a_file_without_a_declared_type() {
    let app = TestBackend::new(true).app();
    let response = upload(
        &app,
        &[
            Part::Text("kind", "side_2"),
            Part::File {
                name: "file",
                file_name: "Side View.JPG",
                content_type: None,
                bytes: b"jpeg",
            },
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["file_name"], "Side-View.jpg");
}

#[tokio::test]
async fn reference_upload_checks_the_kind() {
    let app = TestBackend::new(true).app();
    for kind in ["front view", "../front", &"a".repeat(33)] {
        let response = upload(
            &app,
            &[
                Part::Text("kind", kind),
                image("front.png", "image/png", b"png"),
            ],
        )
        .await;
        assert_detail(response, StatusCode::BAD_REQUEST, "Invalid reference kind").await;
    }
}

#[tokio::test]
async fn reference_upload_needs_both_fields() {
    let app = TestBackend::new(true).app();
    for parts in [
        vec![Part::Text("kind", "front")],
        // An empty kind counts as missing.
        vec![
            Part::Text("kind", ""),
            image("front.png", "image/png", b"png"),
        ],
        vec![image("front.png", "image/png", b"png")],
        // `file` given as text, `kind` as a file.
        vec![Part::Text("kind", "front"), Part::Text("file", "front.png")],
        vec![
            Part::File {
                name: "kind",
                file_name: "kind.txt",
                content_type: None,
                bytes: b"front",
            },
            image("front.png", "image/png", b"png"),
        ],
    ] {
        let response = upload(&app, &parts).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(json_body(response).await["detail"].is_string());
    }

    // Not a form at all.
    let request = authed(Method::POST, "/api/references")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reference_upload_refuses_huge_files() {
    let app = TestBackend::new(true).app();
    let huge = vec![0u8; MAX_UPLOAD_BYTES + 1];
    let response = upload(
        &app,
        &[
            Part::Text("kind", "front"),
            image("front.png", "image/png", &huge),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(json_body(response).await["detail"].is_string());
}

#[test]
fn file_names_are_made_safe() {
    let cases = [
        (Some("front.webp"), "front.webp"),
        (Some("../My Photo (1).PNG"), "My-Photo-1.png"),
        (Some("C:\\Users\\test_user\\face.jpeg"), "face.jpeg"),
        (Some("dir/sub/"), "sub"),
        (Some("顔.webp"), "reference.webp"),
        (Some(".png"), "png"),
        (Some("a..b.png"), "a..b.png"),
        (Some(".."), "reference"),
        (Some(""), "reference"),
        (None, "reference"),
    ];
    for (raw, expected) in cases {
        assert_eq!(safe_filename(raw).unwrap(), expected, "{raw:?}");
    }
}

#[tokio::test]
async fn reference_assets_refuse_paths_outside_the_folder() {
    let backend = TestBackend::new(true);
    let app = backend.app();
    let folder = backend.dir.path().join(DIR_NAME).join("upload-a");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("front.png"), b"png").unwrap();
    std::fs::write(backend.dir.path().join("secret.png"), b"secret").unwrap();
    std::fs::create_dir_all(backend.dir.path().join(DIR_NAME).join("folder.png")).unwrap();

    let invalid = "Invalid reference asset path";
    for uri in [
        "/api/references/",
        "/api/references/references",
        "/api/references/other/upload-a/front.png",
        "/api/references/references/../secret.png",
        "/api/references/references/%2e%2e/secret.png",
        "/api/references/references/upload-a/..%2f..%2fsecret.png",
        "/api/references/references/upload-a%5c..%5c..%5csecret.png",
        "/api/references/%2freferences/upload-a/front.png",
        "/api/references/references/C:/secret.png",
        "/api/references/references/upload-a/front.png:stream.png",
    ] {
        assert_detail(get(&app, uri).await, StatusCode::BAD_REQUEST, invalid).await;
    }
    assert_detail(
        get(&app, "/api/references/references/upload-a/front.txt").await,
        StatusCode::BAD_REQUEST,
        "unsupported_image_type",
    )
    .await;
    for uri in [
        "/api/references/references/upload-a/missing.png",
        "/api/references/references/missing/front.png",
        // A folder is not an asset.
        "/api/references/references/folder.png",
    ] {
        assert_detail(
            get(&app, uri).await,
            StatusCode::NOT_FOUND,
            "Reference asset not found",
        )
        .await;
    }

    // `.` parts and doubled slashes are dropped, as before; case of the
    // extension does not matter.
    std::fs::write(folder.join("side.JPG"), b"jpg").unwrap();
    for uri in [
        "/api/references/references/./upload-a//front.png",
        "/api/references/references/upload-a/side.JPG",
    ] {
        assert_eq!(get(&app, uri).await.status(), StatusCode::OK, "{uri}");
    }
    let response = get(&app, "/api/references/references/upload-a/side.JPG").await;
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
}

#[tokio::test]
async fn reference_assets_answer_head_requests() {
    let backend = TestBackend::new(true);
    let folder = backend.dir.path().join(DIR_NAME).join("upload-a");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("front.png"), b"png").unwrap();
    let response = backend
        .app()
        .oneshot(
            authed(
                Method::HEAD,
                "/api/references/references/upload-a/front.png",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    assert!(bytes(response).await.is_empty());
}

#[tokio::test]
async fn other_methods_get_a_detail() {
    let app = TestBackend::new(true).app();
    assert_detail(
        get(&app, "/api/references").await,
        StatusCode::METHOD_NOT_ALLOWED,
        "Method Not Allowed",
    )
    .await;
}
