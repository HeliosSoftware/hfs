use axum::{Router, response::Html, routing::get};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn mount(fhir_app: Router, hfs_version: &'static str) -> Router {
    Router::new()
        .route(
            "/ui",
            get(move || async move { Html(render_index(hfs_version)) }),
        )
        .route(
            "/ui/version",
            get(move || async move { Html(render_version(hfs_version)) }),
        )
        .fallback_service(fhir_app)
}

fn render_index(hfs_version: &str) -> String {
    let version_block = render_version(hfs_version);

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>HFS</title>
  <script src="https://unpkg.com/htmx.org@2.0.4"></script>
  <style>
    :root {{
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f7f8fb;
      color: #172033;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
    }}
    main {{
      width: min(28rem, calc(100vw - 2rem));
      padding: 2rem;
      border: 1px solid #d9dee8;
      border-radius: 8px;
      background: #ffffff;
      box-shadow: 0 16px 40px rgb(23 32 51 / 8%);
    }}
    h1 {{
      margin: 0 0 0.75rem;
      font-size: 1.6rem;
      line-height: 1.2;
      font-weight: 700;
    }}
    p {{
      margin: 0;
      color: #536078;
      line-height: 1.5;
    }}
    .updated-at {{
      display: block;
      margin-top: 0.35rem;
      font-size: 0.85rem;
      color: #6b7280;
    }}
    strong {{
      color: #0f766e;
    }}
    button {{
      margin-top: 1rem;
      padding: 0.65rem 0.9rem;
      border: 1px solid #bac4d6;
      border-radius: 6px;
      background: #172033;
      color: #ffffff;
      font: inherit;
      cursor: pointer;
    }}
    button:hover {{
      background: #26344f;
    }}
  </style>
</head>
<body>
  <main>
    <h1>Helios FHIR Server</h1>
    {}
    <button
      hx-get="/ui/version"
      hx-target="#version"
      hx-swap="outerHTML"
    >
      Refresh version
    </button>
  </main>
</body>
</html>"##,
        version_block
    )
}

fn render_version(hfs_version: &str) -> String {
    render_version_at(hfs_version, unix_timestamp_seconds())
}

fn render_version_at(hfs_version: &str, checked_at: u64) -> String {
    format!(
        r#"<p id="version">hfs version <strong>{hfs_version}</strong><span class="updated-at">Last checked: {checked_at}</span></p>"#
    )
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_contains_hfs_version() {
        let html = render_index("1.2.3");

        assert!(html.contains("Helios FHIR Server"));
        assert!(html.contains("1.2.3"));
        assert!(html.contains("htmx.org"));
        assert!(html.contains(r#"hx-get="/ui/version""#));
    }

    #[test]
    fn version_partial_contains_hfs_version() {
        let html = render_version_at("1.2.3", 42);

        assert_eq!(
            html,
            r#"<p id="version">hfs version <strong>1.2.3</strong><span class="updated-at">Last checked: 42</span></p>"#
        );
    }
}
