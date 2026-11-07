//! Every layer the workers publish must have a table in the `muted` (viridis, colour-blind safe)
//! palette, which the web and app legends read from `/api/palettes`.

use radar_aux::aqm::AQM_LAYER_SPECS;
use radar_ingest::global_model::Product;
use radar_ingest::hrrr::state::layer_color_key;
use radar_tiler::palettes::load_palette;

#[test]
fn every_published_layer_has_a_muted_table() {
    let mut keys: Vec<(String, &str)> = Vec::new();
    for layer in [
        "radar-hrrr",
        "temperature",
        "feels-like",
        "dewpoint",
        "humidity",
        "wind",
        "cape",
        "precip-type",
        "precip-accum",
        "cloud",
    ] {
        let key = layer_color_key(layer).unwrap_or_else(|| panic!("no color key for {layer}"));
        keys.push((layer.to_string(), key));
    }
    for p in radar_ingest::mrms_qpe::PRODUCTS {
        keys.push((p.layer.to_string(), p.color_key));
    }
    for p in Product::ALL {
        keys.push((p.layer().to_string(), p.color_key()));
    }
    for spec in AQM_LAYER_SPECS {
        keys.push((spec.name.to_string(), spec.color_key));
    }
    let muted = load_palette("muted").unwrap();
    for (layer, key) in keys {
        assert!(
            muted.color_keys().any(|k| k == key),
            "layer {layer}: muted palette has no table for {key}"
        );
    }
}
