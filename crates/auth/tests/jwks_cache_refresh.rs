//! How many fetches an unknown `kid` costs the identity provider.
//!
//! The `kid` comes from an unverified bearer, so the cache must answer a
//! burst of unknown-kid lookups with one fetch per refresh interval, whether
//! that fetch succeeds or fails.

use std::sync::Arc;
use std::time::Duration;

use helios_auth::{AuthError, JwksCache};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KID: &str = "test-key-1";
const JWK_N: &str = "zbeWVBfqrPsdF1phb2EB5hxjz1EMb5WT4MnhMatFnNFquEZIcgXG1DBAUICEZLQM8gRj9kZ83YA1cFhUtkgUx1qMio8YkKweLV6r8RAqnuxrS5Khm4a25M85qB5MhGoCLZgGtla9lJ79mjVN5XBqD7sPL0ENN1GPcnus1I6WF-E-5Uiih-QWsQVlGeUqMH81vTBLemDxvMQv-TiKZP3LHH_T41G1FfyF-rlbkj7WXGkbqG_WvR4FKeRm1X2_RRVw87h2j1lfK4AiUKs4h_DqwqZju3fgOH_0MHj2RKCu0wo0cTx8mQGbLCNSb-sRoWRnWGpGPphI9xVS8znuPB1JYQ";
const JWK_E: &str = "AQAB";

fn jwks_body() -> serde_json::Value {
    json!({
        "keys": [{
            "kty": "RSA",
            "kid": KID,
            "use": "sig",
            "alg": "RS256",
            "n": JWK_N,
            "e": JWK_E,
        }]
    })
}

async fn fetches(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .len()
}

#[tokio::test]
async fn concurrent_unknown_kid_lookups_share_one_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jwks_body())
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;
    let cache = Arc::new(JwksCache::new(&format!("{}/jwks", server.uri()), 60));
    cache.initial_fetch().await.expect("initial fetch");
    assert_eq!(fetches(&server).await, 1);

    // The initial fetch armed the 60 s interval, so the burst arrives inside it.
    let lookups: Vec<_> = (0..50)
        .map(|_| {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move { cache.get_key("unknown").await })
        })
        .collect();
    for lookup in lookups {
        let outcome = lookup.await.expect("lookup task").err();
        assert!(
            matches!(outcome, Some(AuthError::UnknownKid { .. })),
            "{outcome:?}"
        );
    }
    assert_eq!(
        fetches(&server).await,
        1,
        "the initial fetch armed the interval, so the burst cost nothing"
    );

    assert!(
        cache.get_key(KID).await.is_ok(),
        "the known key still resolves"
    );
}

#[tokio::test]
async fn a_burst_after_the_interval_costs_one_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(jwks_body())
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;
    let cache = Arc::new(JwksCache::new(&format!("{}/jwks", server.uri()), 1));
    cache.initial_fetch().await.expect("initial fetch");
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let lookups: Vec<_> = (0..50)
        .map(|_| {
            let cache = Arc::clone(&cache);
            tokio::spawn(async move { cache.get_key("unknown").await })
        })
        .collect();
    for lookup in lookups {
        let outcome = lookup.await.expect("lookup task").err();
        assert!(
            matches!(outcome, Some(AuthError::UnknownKid { .. })),
            "{outcome:?}"
        );
    }
    assert_eq!(
        fetches(&server).await,
        2,
        "fifty concurrent unknown-kid lookups share one fetch"
    );
}

#[tokio::test]
async fn a_failed_fetch_arms_the_interval() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_body()))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let cache = JwksCache::new(&format!("{}/jwks", server.uri()), 1);
    cache.initial_fetch().await.expect("initial fetch");
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let first = cache.get_key("unknown").await.err();
    assert!(
        matches!(first, Some(AuthError::JwksFetchError(_))),
        "the lookup that fetched reports the failure: {first:?}"
    );
    assert_eq!(fetches(&server).await, 2);

    for _ in 0..5 {
        let later = cache.get_key("unknown").await.err();
        assert!(
            matches!(later, Some(AuthError::UnknownKid { .. })),
            "a lookup inside the interval does not fetch again: {later:?}"
        );
    }
    assert_eq!(
        fetches(&server).await,
        2,
        "a failed fetch arms the interval like a successful one"
    );
    assert!(
        cache.get_key(KID).await.is_ok(),
        "the keys from the last good fetch are still served"
    );
}
