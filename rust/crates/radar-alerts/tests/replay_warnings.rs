use std::collections::BTreeMap;

use radar_alerts::replay::{Bulletin, Case, normalize};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../web/src/shared/replayWarnings.fixture.json"
    ))
    .unwrap()
}

fn input() -> (Value, Vec<Value>, BTreeMap<String, Bulletin>) {
    let fixture = fixture();
    let mut versions = fixture["history"]["features"].as_array().unwrap().clone();
    let mut bulletins = BTreeMap::new();
    for feature in &mut versions {
        let p = feature["properties"].as_object_mut().unwrap();
        let id = p["product_id"].as_str().unwrap().to_owned();
        let text = format!(
            "/O.{}.KJAN.{}.W.{:04}.000000T0000Z-230325T0030Z/\n{}\n{}\n$$\n/O.CON.KJAN.TO.W.9999.000000T0000Z-230325T0030Z/\nTORNADO EMERGENCY PARTICULARLY DANGEROUS SITUATION\n$$",
            p["status"].as_str().unwrap(),
            p["phenomena"].as_str().unwrap(),
            p["eventid"].as_i64().unwrap(),
            if p["is_emergency"] == true {
                "TORNADO EMERGENCY"
            } else {
                ""
            },
            if p["is_pds"] == true {
                "PARTICULARLY DANGEROUS SITUATION"
            } else {
                ""
            },
        );
        bulletins.insert(
            id.clone(),
            Bulletin::new(format!("https://example.test/{id}"), text),
        );
        p.insert("utc_issue".into(), json!("2023-03-24T22:50:00Z"));
        p.insert("utc_expire".into(), json!("2023-03-25T00:30:00Z"));
        p.insert("max_is_emergency".into(), json!(true));
        p.insert("max_is_pds".into(), json!(true));
        p.remove("is_emergency");
        p.remove("is_pds");
        p.remove("event");
    }
    versions[4]["properties"]["utc_polygon_end"] = json!("2023-03-25T00:30:00Z");
    let mut original = versions[2].clone();
    original["properties"]["product_id"] = json!("202303250000-KJAN-WWUS54-SVSJAN");
    let original_id = original["properties"]["product_id"].as_str().unwrap();
    bulletins.insert(
        original_id.into(),
        Bulletin::new(
            "https://example.test/original".into(),
            "/O.CON.KJAN.TO.W.0001.000000T0000Z-230325T0030Z/\nordinary\n$$".into(),
        ),
    );
    versions.push(original);
    let mut zero = versions[0].clone();
    zero["properties"]["utc_polygon_end"] = zero["properties"]["utc_polygon_begin"].clone();
    versions.push(zero);
    let mut seed = json!({"type":"FeatureCollection", "features":[versions[0].clone()]});
    let p = seed["features"][0]["properties"].as_object_mut().unwrap();
    for (old, new) in [
        ("utc_polygon_begin", "polygon_begin"),
        ("utc_polygon_end", "polygon_end"),
        ("utc_issue", "issue"),
        ("utc_expire", "expire"),
    ] {
        let value = p.remove(old).unwrap();
        p.insert(new.into(), value);
    }
    (seed, versions, bulletins)
}

#[test]
fn normalize_matches_both_clients_and_is_order_independent() {
    let (seed, mut versions, bulletins) = input();
    let case = Case::named("rolling-fork-2023").unwrap();
    let history = normalize(&case, &seed, &versions, bulletins.clone(), vec![]).unwrap();
    let expected = fixture();
    assert_eq!(
        serde_json::to_value(&history.features).unwrap(),
        expected["history"]["features"]
    );
    assert_eq!(
        serde_json::to_value(&history.seed).unwrap(),
        expected["history"]["seed"]
    );
    versions.reverse();
    assert_eq!(
        history,
        normalize(&case, &seed, &versions, bulletins, vec![]).unwrap()
    );
}

#[test]
fn incomplete_seed_history_or_missing_bulletin_fails() {
    let (seed, mut versions, mut bulletins) = input();
    let case = Case::named("rolling-fork-2023").unwrap();
    bulletins.clear();
    assert!(normalize(&case, &seed, &versions, bulletins, vec![]).is_err());
    let (_, _, bulletins) = input();
    versions.retain(|f| f["properties"]["utc_polygon_begin"] != "2023-03-24T22:50:00Z");
    assert!(normalize(&case, &seed, &versions, bulletins, vec![]).is_err());
}

#[test]
fn later_correction_does_not_apply_emergency_wording_retroactively() {
    let (seed, mut versions, mut bulletins) = input();
    let old = "202303250000-KJAN-WWUS54-SVSJAN-CCA";
    let new = "202303250001-KJAN-WWUS54-SVSJAN-CCA";
    versions[2]["properties"]["product_id"] = json!(new);
    let bulletin = bulletins.remove(old).unwrap();
    bulletins.insert(new.into(), bulletin);
    let history = normalize(
        &Case::named("rolling-fork-2023").unwrap(),
        &seed,
        &versions,
        bulletins,
        vec![],
    )
    .unwrap();
    let earlier = history
        .features
        .iter()
        .find(|f| f.properties.product_id == "202303250000-KJAN-WWUS54-SVSJAN")
        .unwrap();
    assert!(!earlier.properties.is_emergency);
    assert_eq!(earlier.properties.utc_polygon_end, "2023-03-25T00:01:00Z");
    let corrected = history
        .features
        .iter()
        .find(|f| f.properties.product_id == new)
        .unwrap();
    assert!(corrected.properties.is_emergency);
    assert_eq!(
        corrected.properties.utc_polygon_begin,
        "2023-03-25T00:01:00Z"
    );
    assert!(corrected.id.ends_with("/2023-03-25T00:00:00Z"));
}

#[test]
fn conflicting_inferior_versions_fail_in_both_orders() {
    let (seed, mut versions, bulletins) = input();
    let mut conflicting = versions[6].clone();
    conflicting["geometry"]["coordinates"][0][1][0] = json!(-89.5);
    versions.push(conflicting);
    for _ in 0..2 {
        let error = normalize(
            &Case::named("rolling-fork-2023").unwrap(),
            &seed,
            &versions,
            bulletins.clone(),
            vec![],
        )
        .unwrap_err();
        assert!(error.to_string().contains("conflicting copies"));
        versions.reverse();
    }
}

#[test]
fn corrected_new_without_original_history_fails_closed() {
    let (seed, mut versions, mut bulletins) = input();
    let original = versions[4]["properties"]["product_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let corrected = "202303242347-KJAN-WUUS54-SVRJAN-CCA";
    versions[4]["properties"]["product_id"] = json!(corrected);
    let bulletin = bulletins.remove(&original).unwrap();
    bulletins.insert(corrected.into(), bulletin);
    let error = normalize(
        &Case::named("rolling-fork-2023").unwrap(),
        &seed,
        &versions,
        bulletins,
        vec![],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("incomplete corrected NEW history")
    );
}

#[test]
fn zero_duration_cancellation_still_ends_the_previous_version() {
    let (seed, mut versions, bulletins) = input();
    versions[5]["properties"]["utc_polygon_end"] =
        versions[5]["properties"]["utc_polygon_begin"].clone();
    let history = normalize(
        &Case::named("rolling-fork-2023").unwrap(),
        &seed,
        &versions,
        bulletins,
        vec![],
    )
    .unwrap();
    let prior = history
        .features
        .iter()
        .find(|f| f.properties.eventid == 2 && f.properties.status == "NEW")
        .unwrap();
    assert_eq!(prior.properties.utc_polygon_end, "2023-03-25T00:05:00Z");
    assert!(
        history
            .features
            .iter()
            .any(|f| f.properties.status == "CAN")
    );
}
