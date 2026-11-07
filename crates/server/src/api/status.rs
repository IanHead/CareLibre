//! Human-facing status page served at `/`.
//!
//! Deliberately dependency-free: a single HTML document rendered at request
//! time so the status shown is live, not cached from startup.

use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::Html,
};
use sqlx::PgPool;

use crate::{state::AppState, util::format_duration};

const DATABASE_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// `GET /` — status + client connection guide.
pub async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ([(header::HeaderName, &'static str); 1], Html<String>) {
    let database = database_status(&state.db).await;

    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("127.0.0.1:{}", state.config.bind_addr.port()));

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "http" || *value == "https")
        .unwrap_or("http");

    let base_url = format!("{scheme}://{}/api/v1", escape_html(&host));

    let html = TEMPLATE
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
        .replace("{{ENVIRONMENT}}", state.config.environment.as_str())
        .replace("{{UPTIME}}", &format_duration(state.started_at.elapsed()))
        .replace(
            "{{TIME}}",
            &chrono::Utc::now()
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string(),
        )
        .replace("{{DB_DOT}}", database.dot())
        .replace("{{DB_STATE}}", &database.label())
        .replace("{{BASE_URL}}", &base_url);

    ([(header::CACHE_CONTROL, "no-store")], Html(html))
}

enum DatabaseStatus {
    Up { latency_ms: u128 },
    Down,
}

impl DatabaseStatus {
    const fn dot(&self) -> &'static str {
        match self {
            Self::Up { .. } => "ok",
            Self::Down => "bad",
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Up { latency_ms } => format!("Operational · {latency_ms} ms"),
            Self::Down => "Unavailable".to_owned(),
        }
    }
}

async fn database_status(db: &PgPool) -> DatabaseStatus {
    let started = Instant::now();

    match tokio::time::timeout(
        DATABASE_CHECK_TIMEOUT,
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(db),
    )
    .await
    {
        Ok(Ok(_)) => DatabaseStatus::Up {
            latency_ms: started.elapsed().as_millis(),
        },
        Ok(Err(err)) => {
            // Log the detail; never render it. Driver errors can include the
            // connection string and therefore the password.
            tracing::warn!(error = %err, "status page database check failed");
            DatabaseStatus::Down
        }
        Err(_) => {
            tracing::warn!("status page database check timed out");
            DatabaseStatus::Down
        }
    }
}

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CareLibre Server · {{VERSION}}</title>
<link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>🌿</text></svg>">
<style>
:root {
  color-scheme: light dark;
  --bg: #f5f7f8;
  --card: #ffffff;
  --text: #171a1f;
  --muted: #68707d;
  --border: #e2e6ea;
  --ok: #128a4c;
  --bad: #cf2f2f;
  --accent: #0d6b62;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0f1216;
    --card: #161a20;
    --text: #e8eaef;
    --muted: #98a1b0;
    --border: #262c37;
    --ok: #3ddc84;
    --bad: #ff6b6b;
    --accent: #6ee7d5;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 15px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
}
main { max-width: 900px; margin: 0 auto; padding: 48px 20px 64px; }
header h1 { margin: 0; font-size: 30px; letter-spacing: -0.02em; }
header h1 span { color: var(--muted); font-weight: 400; }
.sub { margin: 6px 0 0; color: var(--muted); font-size: 13px; }
section { margin-top: 34px; }
h2 {
  font-size: 12px; text-transform: uppercase; letter-spacing: .09em;
  color: var(--muted); margin: 0 0 12px; font-weight: 600;
}
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 12px; }
.card { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 16px 18px; }
.card h3 {
  margin: 0 0 7px; font-size: 11px; text-transform: uppercase;
  letter-spacing: .08em; color: var(--muted); font-weight: 600;
}
.card p { margin: 0; font-size: 15px; }
.dot { display: inline-block; width: 9px; height: 9px; border-radius: 50%; margin-right: 7px; }
.dot.ok { background: var(--ok); }
.dot.bad { background: var(--bad); }
table {
  width: 100%; border-collapse: collapse; background: var(--card);
  border: 1px solid var(--border); border-radius: 10px; overflow: hidden;
}
th, td { text-align: left; padding: 11px 16px; border-bottom: 1px solid var(--border); font-size: 14px; }
tr:last-child th, tr:last-child td { border-bottom: 0; }
th { width: 32%; color: var(--muted); font-weight: 500; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .92em; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
pre {
  background: var(--card); border: 1px solid var(--border);
  border-radius: 10px; padding: 14px 16px; overflow: auto; margin: 14px 0 0;
}
pre code { font-size: 12.5px; }
ol { margin: 0; padding-left: 20px; }
ol li { margin-bottom: 7px; }
footer { margin-top: 44px; padding-top: 18px; border-top: 1px solid var(--border); color: var(--muted); font-size: 13px; }
footer p { margin: 8px 0 0; }
</style>
</head>
<body>
<main>
  <header>
    <h1>CareLibre <span>Server</span></h1>
    <p class="sub">v{{VERSION}} · {{ENVIRONMENT}}</p>
  </header>

  <section>
    <h2>Status</h2>
    <div class="grid">
      <div class="card">
        <h3>API</h3>
        <p><span class="dot ok"></span>Operational</p>
      </div>
      <div class="card">
        <h3>Database</h3>
        <p><span class="dot {{DB_DOT}}"></span>{{DB_STATE}}</p>
      </div>
      <div class="card">
        <h3>Uptime</h3>
        <p>{{UPTIME}}</p>
      </div>
      <div class="card">
        <h3>Server time</h3>
        <p>{{TIME}}</p>
      </div>
    </div>
  </section>

  <section>
    <h2>Connect a client</h2>
    <table>
      <tr><th>Base URL</th><td><code>{{BASE_URL}}</code></td></tr>
      <tr><th>OpenAPI spec</th><td><a href="/api-docs/openapi.json"><code>/api-docs/openapi.json</code></a></td></tr>
      <tr><th>Interactive docs</th><td><a href="/docs"><code>/docs</code></a></td></tr>
      <tr><th>Health</th><td><a href="/healthz"><code>/healthz</code></a> · <a href="/readyz"><code>/readyz</code></a></td></tr>
    </table>
  </section>

  <section>
    <h2>Authenticate</h2>
    <ol>
      <li>Call <code>POST {{BASE_URL}}/auth/login</code> with <code>email</code> and <code>password</code> (plus <code>organisation_slug</code> if the account belongs to more than one organisation).</li>
      <li>Send <code>Authorization: Bearer &lt;access_token&gt;</code> with every request. Access tokens last 15 minutes.</li>
      <li>On <code>401</code>, call <code>POST {{BASE_URL}}/auth/refresh</code> with the refresh token and retry once. Refresh tokens are single-use.</li>
    </ol>
    <pre><code>curl -sX POST {{BASE_URL}}/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"jane@example.com","password":"...","organisation_slug":"example-care"}'</code></pre>
  </section>

  <footer>
    <a href="https://github.com/IanHead/CareLibre">Repository</a> ·
    <a href="/admin">Administration</a> ·
    <a href="/docs">API documentation</a>
    <p>AGPL-3.0-or-later</p>
  </footer>
</main>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_dangerous_characters() {
        assert_eq!(
            escape_html(r#"<script>"x" & 'y'"#),
            "&lt;script&gt;&quot;x&quot; &amp; &#39;y&#39;"
        );
    }
}
