//! Subscriptions operator page (#580) — read-only, per Brett's design.
//!
//! One screen over the engine's live inventory: four status cards (active /
//! failing / idle / delivered in 24 h) and a table of every subscription with
//! its channel, status, 24-hour delivery count, event counter, and
//! consecutive failure streak. The data arrives through the process-global
//! [`helios_observability::subscriptions`] provider the server registers when
//! the engine is enabled — the page renders an explained unavailable state
//! (naming `HFS_SUBSCRIPTIONS_ENABLED` and the `subscriptions` build feature)
//! when it is not; the sidebar entry is always present (#767).
//!
//! The design's 24-hour sparkline needs a per-window time series the
//! [`SubscriptionsSnapshot`](helios_observability::subscriptions::SubscriptionsSnapshot)
//! does not carry; the column shows the real count until the engine grows
//! that series (#555's rule: real figures or none).

use askama::Template;
use axum::{extract::RawQuery, extract::State, response::Response};

use crate::i18n::{I18n, RequestLocale};
use crate::{RequestTenant, RequestVersion, WebState, grouped, query_value, render};

/// One table row, ready to print.
pub struct SubscriptionRowView {
    pub id: String,
    /// The topic's last path segment (`encounter-start`), for the compact cell.
    pub topic: String,
    /// The full canonical URL, for the cell's tooltip.
    pub topic_url: String,
    pub channel: String,
    pub endpoint: String,
    /// The status chip's label (localized keys resolve in the template) —
    /// `active` / `error` / `requested` / `off`, or the synthetic `idle` for a
    /// websocket subscription nobody is connected to.
    pub state: &'static str,
    pub sent: String,
    /// Notifications delivered in the last 24 hours (#586).
    pub last24: String,
    /// Consecutive delivery failures; `—` when the channel has no delivery
    /// (an idle websocket).
    pub streak: String,
}

/// The four headline cards.
pub struct SubscriptionCards {
    pub failing: usize,
    pub idle: usize,
    pub active: usize,
    /// Notifications delivered across the tenant in the last 24 hours.
    pub delivered: String,
    /// First-try success rate over that window ("96.4"), when anything was
    /// delivered — absent means no deliveries, not a fabricated 100%.
    pub rate: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/subscriptions.html")]
pub struct SubscriptionsPage {
    pub status: crate::Status,
    pub i18n: I18n,
    pub active_page: &'static str,
    /// The engine is registered; `false` renders the explained unavailable
    /// state instead of an empty dashboard.
    pub available: bool,
    pub cards: SubscriptionCards,
    pub rows: Vec<SubscriptionRowView>,
    pub sort: &'static str,
}

/// `GET /ui/subscriptions` — the read-only operator dashboard. `?sort=` picks
/// the table order (`status` default: worst first; `sent`, `fails`).
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    RawQuery(query): RawQuery,
) -> Response {
    let snapshot = helios_observability::subscriptions::snapshot(&rt.id);
    let available = snapshot.is_some();
    let sort = match query_value(query.as_deref(), "sort").as_deref() {
        Some("sent") => "sent",
        Some("fails") => "fails",
        _ => "status",
    };

    let source_rows = snapshot.map(|snap| snap.rows).unwrap_or_default();
    // Tenant-wide delivery figures for the DELIVERED IN 24 H card (#586).
    let delivered_total: u64 = source_rows.iter().map(|r| r.delivered_24h).sum();
    let first_try_total: u64 = source_rows.iter().map(|r| r.first_try_24h).sum();
    let rate = (delivered_total > 0).then(|| {
        format!(
            "{:.1}",
            first_try_total as f64 * 100.0 / delivered_total as f64
        )
    });

    let mut rows: Vec<SubscriptionRowView> = source_rows
        .into_iter()
        .map(|row| {
            // A websocket subscription with no connected clients produces
            // notifications that go nowhere — the design's "0 clients" state
            // outranks the resource's own `active`.
            let idle = row.ws_clients == Some(0);
            let state = if idle {
                "idle"
            } else {
                match row.status.as_str() {
                    "active" => "active",
                    "error" => "error",
                    "requested" => "requested",
                    _ => "off",
                }
            };
            let topic = row
                .topic_url
                .rsplit('/')
                .next()
                .unwrap_or(&row.topic_url)
                .to_string();
            SubscriptionRowView {
                id: row.id,
                topic,
                topic_url: row.topic_url,
                channel: row.channel_type,
                endpoint: row.endpoint.unwrap_or_default(),
                state,
                sent: grouped(row.events_since_start),
                last24: grouped(row.delivered_24h),
                streak: if idle {
                    "—".to_string()
                } else {
                    row.consecutive_failures.to_string()
                },
            }
        })
        .collect();

    // Worst first by default: a failing subscription is the row the operator
    // came for.
    let severity = |state: &str| match state {
        "error" => 0,
        "idle" => 1,
        "requested" => 2,
        "active" => 3,
        _ => 4,
    };
    match sort {
        "sent" => rows.sort_by(|a, b| {
            parse_grouped(&b.sent)
                .cmp(&parse_grouped(&a.sent))
                .then_with(|| a.id.cmp(&b.id))
        }),
        "fails" => rows.sort_by(|a, b| {
            b.streak
                .parse::<u64>()
                .unwrap_or(0)
                .cmp(&a.streak.parse::<u64>().unwrap_or(0))
                .then_with(|| a.id.cmp(&b.id))
        }),
        _ => rows.sort_by(|a, b| {
            severity(a.state)
                .cmp(&severity(b.state))
                .then_with(|| parse_grouped(&b.sent).cmp(&parse_grouped(&a.sent)))
                .then_with(|| a.id.cmp(&b.id))
        }),
    }

    let cards = SubscriptionCards {
        failing: rows.iter().filter(|r| r.state == "error").count(),
        idle: rows.iter().filter(|r| r.state == "idle").count(),
        active: rows.iter().filter(|r| r.state == "active").count(),
        delivered: grouped(delivered_total),
        rate,
    };

    render(SubscriptionsPage {
        status: crate::current_status(&state, rv.0, &rt),
        i18n: I18n::new(locale),
        active_page: "subscriptions",
        available,
        cards,
        rows,
        sort,
    })
}

/// Reads a `grouped()` figure back ("4,182" → 4182) — the rows carry display
/// strings, and the card sums reuse them rather than threading raw values.
fn parse_grouped(s: &str) -> u64 {
    s.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}
