//! Shared HTML helpers for the server-rendered admin area.
//!
//! No template engine and no client-side framework: pages are plain HTML with
//! inline CSS, which keeps the deployment a single binary.

pub fn escape(value: &str) -> String {
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

/// Everything needed to render the signed-in chrome.
pub struct Who<'a> {
    pub email: &'a str,
    pub organisation: &'a str,
    pub role: &'a str,
    pub capabilities: crate::permissions::CapabilitySet,
}

/// Full page shell. `active` is the nav href to highlight.
pub fn layout(title: &str, active: &str, who: Option<&Who<'_>>, body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 8_000);

    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<link rel=\"icon\" href=\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>🌿</text></svg>\">\n");
    out.push_str("<title>");
    out.push_str(&escape(title));
    out.push_str(" · CareLibre Admin</title>\n<style>\n");
    out.push_str(STYLE);
    out.push_str("</style>\n</head>\n<body>\n");

    if let Some(who) = who {
        let capabilities = who.capabilities;
        out.push_str("<div class=\"shell\">\n<aside class=\"sidebar\">\n");
        out.push_str("<a class=\"brand\" href=\"/admin\">CareLibre <span>Admin</span></a>\n");
        out.push_str("<input id=\"nav-filter\" class=\"nav-filter\" type=\"search\" placeholder=\"Filter menu\" aria-label=\"Filter menu\">\n");
        out.push_str("<nav class=\"nav\" id=\"admin-nav\">\n");

        out.push_str("<div class=\"nav-group\"><h2>Home</h2>");
        out.push_str(&nav_item("/admin", "Dashboard", active));
        if capabilities.contains(crate::permissions::Capability::SettingsManage) {
            out.push_str(&nav_item("/admin/onboarding", "Setup", active));
        }
        out.push_str("</div>");

        let mut access = String::new();
        if capabilities.contains(crate::permissions::Capability::UsersManage) {
            access.push_str(&nav_item("/admin/users", "Users", active));
            access.push_str(&nav_item("/admin/invitations", "Invitations", active));
            access.push_str(&nav_item("/admin/sessions", "Sessions", active));
        } else if capabilities.contains(crate::permissions::Capability::AuditRead) {
            access.push_str(&nav_item("/admin/sessions", "Sessions", active));
        }
        if capabilities.contains(crate::permissions::Capability::PermissionSetsManage) {
            access.push_str(&nav_item(
                "/admin/permission-sets",
                "Permission sets",
                active,
            ));
        }
        if !access.is_empty() {
            out.push_str("<div class=\"nav-group\"><h2>Access</h2>");
            out.push_str(&access);
            out.push_str("</div>");
        }

        let mut data = String::new();
        if capabilities.contains(crate::permissions::Capability::RelationshipsRead) {
            data.push_str(&nav_item("/admin/relationships", "Relationships", active));
        }
        if capabilities.contains(crate::permissions::Capability::DataImport) {
            data.push_str(&nav_item("/admin/import", "Import", active));
        }
        if capabilities.contains(crate::permissions::Capability::CustomFieldsManage) {
            data.push_str(&nav_item("/admin/custom-fields", "Custom fields", active));
        }
        if !data.is_empty() {
            out.push_str("<div class=\"nav-group\"><h2>Data</h2>");
            out.push_str(&data);
            out.push_str("</div>");
        }

        out.push_str("<div class=\"nav-group\"><h2>System</h2>");
        if capabilities.contains(crate::permissions::Capability::SettingsManage) {
            out.push_str(&nav_item("/admin/organisation", "Organisation", active));
        }
        if capabilities.contains(crate::permissions::Capability::SchemaView) {
            out.push_str(&nav_item("/admin/schema", "Database", active));
        }
        out.push_str(&nav_item("/", "Status page", active));
        out.push_str(&nav_item("/docs", "API docs", active));
        out.push_str("</div>");

        out.push_str("</nav>\n<div class=\"sidebar-foot\">\n");
        out.push_str("<span class=\"who-org\">");
        out.push_str(&escape(who.organisation));
        out.push_str("</span>\n<span class=\"who-mail\">");
        out.push_str(&escape(who.email));
        out.push_str("</span>\n<span class=\"pill\">");
        out.push_str(&escape(who.role));
        out.push_str("</span>\n<form method=\"post\" action=\"/admin/logout\"><button class=\"link-button\" type=\"submit\">Sign out</button></form>\n");
        out.push_str("</div>\n</aside>\n<main class=\"content\">\n");
    } else {
        out.push_str("<main class=\"public\">\n");
    }

    out.push_str(body);
    out.push_str("\n<footer><a href=\"/\">Status</a> · <a href=\"/docs\">API docs</a> · <span>v");
    out.push_str(env!("CARGO_PKG_VERSION"));
    out.push_str("</span></footer>\n</main>\n");

    if who.is_some() {
        out.push_str(
            "<script>(function(){var input=document.getElementById('nav-filter');\
             if(!input)return;input.addEventListener('input',function(){var q=input.value.trim().toLowerCase();\
             document.querySelectorAll('.nav-group').forEach(function(group){var any=false;\
             group.querySelectorAll('a.nav-item').forEach(function(link){var match=link.textContent.toLowerCase().indexOf(q)!==-1;\
             link.style.display=match?'':'none';if(match)any=true;});group.style.display=any?'':'none';});});})();</script>\n",
        );
        out.push_str("</div>\n");
    }
    out.push_str("</body>\n</html>\n");

    out
}

fn nav_item(href: &str, label: &str, active: &str) -> String {
    if href == active {
        format!("<a class=\"nav-item active\" href=\"{href}\">{label}</a>")
    } else {
        format!("<a class=\"nav-item\" href=\"{href}\">{label}</a>")
    }
}

pub fn page_head(title: &str, subtitle: &str) -> String {
    format!(
        "<div class=\"page-head\"><div><h1>{}</h1><p class=\"muted\">{}</p></div></div>",
        escape(title),
        escape(subtitle)
    )
}

pub fn error_page(title: &str, message: &str) -> String {
    let body = format!(
        "<div class=\"page-head\"><h1>{}</h1><p class=\"muted\">{}</p></div>\
         <p><a href=\"/admin\">Return to the dashboard</a></p>",
        escape(title),
        escape(message)
    );
    layout(title, "", None, &body)
}

const STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #f5f7f8;
  --card: #ffffff;
  --text: #171a1f;
  --muted: #68707d;
  --border: #e2e6ea;
  --accent: #0d6b62;
  --accent-soft: #e6f4f1;
  --ok: #128a4c;
  --bad: #cf2f2f;
  --warn: #b45309;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0f1216;
    --card: #161a20;
    --text: #e8eaef;
    --muted: #98a1b0;
    --border: #262c37;
    --accent: #6ee7d5;
    --accent-soft: #12312d;
    --ok: #3ddc84;
    --bad: #ff6b6b;
    --warn: #fbbf24;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--text);
  font: 15px/1.55 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
}
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
code {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: .92em;
}
.shell { display: grid; grid-template-columns: 250px minmax(0, 1fr); min-height: 100vh; }
.sidebar {
  position: sticky; top: 0; height: 100vh; overflow-y: auto;
  background: var(--card); border-right: 1px solid var(--border);
  padding: 18px 14px; display: flex; flex-direction: column; gap: 14px;
}
.brand { font-size: 17px; font-weight: 650; letter-spacing: -0.01em; color: var(--text); white-space: nowrap; padding: 0 6px; }
.brand span { color: var(--muted); font-weight: 400; }
.nav-filter {
  width: 100%; padding: 8px 10px; border-radius: 7px; border: 1px solid var(--border);
  background: var(--bg); color: var(--text); font: inherit; font-size: 13px;
}
.nav-filter:focus { outline: 2px solid var(--accent); outline-offset: 1px; border-color: transparent; }
.nav { display: flex; flex-direction: column; gap: 18px; flex: 1; }
.nav-group h2 { margin: 0 0 6px; padding: 0 8px; font-size: 10px; }
.nav-item { display: block; padding: 7px 9px; border-radius: 7px; font-size: 13.5px; color: var(--muted); }
.nav-item:hover { background: var(--bg); text-decoration: none; }
.nav-item.active { background: var(--accent-soft); color: var(--accent); font-weight: 600; }
.sidebar-foot {
  border-top: 1px solid var(--border); padding-top: 12px;
  display: flex; flex-direction: column; gap: 3px; font-size: 12px; color: var(--muted);
}
.sidebar-foot .who-org { color: var(--text); font-weight: 600; font-size: 13px; }
.who-mail { overflow: hidden; text-overflow: ellipsis; }
@media (max-width: 860px) {
  .shell { grid-template-columns: 1fr; }
  .sidebar { position: static; height: auto; border-right: 0; border-bottom: 1px solid var(--border); }
  .nav { flex-direction: row; flex-wrap: wrap; gap: 12px; }
  .nav-group { flex: 1 1 160px; }
  .sidebar-foot { flex-direction: row; flex-wrap: wrap; align-items: center; gap: 8px; }
}
.pill {
  border: 1px solid var(--border); border-radius: 999px; padding: 1px 9px;
  font-size: 11px; text-transform: uppercase; letter-spacing: .05em;
}
.link-button {
  background: none; border: 0; padding: 0; color: var(--accent);
  font: inherit; font-size: 13px; cursor: pointer;
}
.content { padding: 34px 36px 60px; width: 100%; max-width: 1180px; margin: 0 auto; }
.public { max-width: 900px; margin: 0 auto; padding: 48px 20px 60px; }
.page-head { display: flex; align-items: flex-start; justify-content: space-between; margin-bottom: 24px; }
h1 { margin: 0; font-size: 24px; letter-spacing: -0.02em; }
h2 { font-size: 12px; text-transform: uppercase; letter-spacing: .09em; color: var(--muted); margin: 34px 0 12px; font-weight: 600; }
h3 { margin: 0 0 6px; font-size: 12px; text-transform: uppercase; letter-spacing: .07em; color: var(--muted); }
.muted { color: var(--muted); }
.sub { margin: 4px 0 0; color: var(--muted); font-size: 13px; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; }
.card { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 16px 18px; }
.card p { margin: 0; }
.card .big { font-size: 26px; font-weight: 650; letter-spacing: -0.02em; }
table { width: 100%; border-collapse: collapse; background: var(--card); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; }
th, td { text-align: left; padding: 10px 14px; border-bottom: 1px solid var(--border); font-size: 13.5px; vertical-align: top; }
tr:last-child td { border-bottom: 0; }
th { color: var(--muted); font-weight: 500; font-size: 12px; text-transform: uppercase; letter-spacing: .05em; }
tbody tr:hover { background: var(--bg); }
.badge {
  display: inline-block; padding: 1px 7px; border-radius: 999px; font-size: 10.5px;
  text-transform: uppercase; letter-spacing: .05em; border: 1px solid var(--border);
}
.badge.pk { color: var(--accent); border-color: var(--accent); }
.badge.fk { color: var(--warn); border-color: var(--warn); }
.badge.id { color: var(--muted); }
.badge.yes { color: var(--bad); border-color: var(--bad); }
.badge.no { color: var(--muted); }
.form { max-width: 420px; background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 24px; }
.form label { display: block; font-size: 12px; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); margin: 16px 0 6px; }
.form label:first-of-type { margin-top: 0; }
.form input {
  width: 100%; padding: 9px 11px; border-radius: 7px; border: 1px solid var(--border);
  background: var(--bg); color: var(--text); font: inherit; font-size: 14px;
}
.form input:focus { outline: 2px solid var(--accent); outline-offset: 1px; border-color: transparent; }
.form button {
  margin-top: 20px; width: 100%; padding: 10px 14px; border-radius: 7px; border: 0;
  background: var(--accent); color: #fff; font: inherit; font-weight: 600; cursor: pointer;
}
@media (prefers-color-scheme: dark) { .form button { color: #062521; } }
.alert { background: color-mix(in srgb, var(--bad) 12%, var(--card)); border: 1px solid var(--bad); color: var(--text); border-radius: 8px; padding: 10px 12px; font-size: 13px; margin-bottom: 16px; }
.erd { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 10px; overflow: auto; }
.erd svg { display: block; min-width: 900px; }
.erd-card { fill: var(--card); stroke: var(--border); }
.erd-title { fill: var(--text); font-weight: 650; font-size: 13px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.erd-col { fill: var(--text); font-size: 11.5px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.erd-type { fill: var(--muted); font-size: 10.5px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.erd-rule { stroke: var(--border); stroke-width: 1; }
.erd-edge { stroke: var(--muted); fill: none; stroke-width: 1.3; opacity: .6; }
.erd-pk { fill: var(--accent); }
.erd-fk { fill: var(--warn); }
.network { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 10px; overflow: auto; }
.network svg { display: block; }
.network-node.contact { fill: var(--accent); }
.network-node.organisation { fill: var(--warn); }
.network-node.focus { stroke: var(--text); stroke-width: 3; }
.network-edge { stroke: var(--muted); stroke-width: .45; opacity: .07; }
.network-edge.near { stroke-width: 1.1; opacity: .35; }
.network-label { fill: var(--text); font-size: 11px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.network-label.small { font-size: 9px; fill: var(--muted); }
.network-role { fill: var(--text); font-size: 9px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; paint-order: stroke; stroke: var(--card); stroke-width: 3px; stroke-linejoin: round; }
.schema-card { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 16px 18px; margin-bottom: 14px; }
.schema-card h3 { font-size: 14px; text-transform: none; letter-spacing: 0; color: var(--text); font-family: ui-monospace, monospace; }
.schema-card table { border: 0; background: transparent; }
.schema-card th, .schema-card td { padding: 6px 10px 6px 0; border-bottom: 1px solid var(--border); font-size: 13px; }
.schema-card th { font-size: 11px; }
.schema-card tbody tr:hover { background: transparent; }
.schema-card ul { margin: 6px 0 0; padding-left: 18px; font-size: 12.5px; color: var(--muted); }
.schema-card ul code { color: var(--text); }
footer { margin-top: 46px; padding-top: 18px; border-top: 1px solid var(--border); color: var(--muted); font-size: 12.5px; display: flex; gap: 8px; }
.button {
  display: inline-block; background: var(--accent); color: #062521;
  padding: 8px 14px; border-radius: 7px; font-weight: 600; font-size: 13.5px;
}
.button:hover { text-decoration: none; filter: brightness(1.06); }
.notice {
  background: color-mix(in srgb, var(--accent) 10%, var(--card));
  border: 1px solid var(--accent); border-radius: 8px;
  padding: 12px 14px; font-size: 13px; margin-bottom: 18px;
}
.warn-box {
  background: color-mix(in srgb, var(--warn) 12%, var(--card));
  border: 1px solid var(--warn); border-radius: 8px;
  padding: 12px 14px; font-size: 13px; margin-bottom: 18px;
}
.danger-box {
  background: color-mix(in srgb, var(--bad) 8%, var(--card));
  border: 1px solid var(--bad); border-radius: 10px;
  padding: 16px 18px; font-size: 13.5px;
}
.danger-box p { margin: 0 0 10px; }
.hint { margin: 4px 0 0; font-size: 12px; color: var(--muted); }
.form.wide { max-width: 760px; }
.form select, .form textarea {
  width: 100%; padding: 9px 11px; border-radius: 7px; border: 1px solid var(--border);
  background: var(--bg); color: var(--text); font: inherit; font-size: 14px;
}
.form textarea { resize: vertical; }
.form .checkbox { display: flex; align-items: center; gap: 8px; margin-top: 16px; }
.form .checkbox input { width: auto; }
.form .checkbox label {
  margin: 0; text-transform: none; letter-spacing: 0;
  font-size: 14px; color: var(--text);
}
.form button.secondary {
  background: var(--card); color: var(--text); border: 1px solid var(--border); margin-top: 4px;
}
.form button.danger { background: var(--bad); color: #fff; }
.grid-2 { display: grid; grid-template-columns: 1fr 1fr; gap: 0 16px; }
@media (max-width: 640px) { .grid-2 { grid-template-columns: 1fr; } }
"#;
