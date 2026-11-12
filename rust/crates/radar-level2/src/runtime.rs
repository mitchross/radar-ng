//! Process wiring for the viewport API and its publisher worker. Both stay off unless
//! `LEVEL2_VIEWPORT_ENABLED=true`, on top of the numeric and tile gates.

use crate::{
    catalogue::Catalogue,
    leases::Pool,
    pgstore::PgStore,
    publish::{Clock, Publisher},
    service::Level2Service,
    tile::{Limits, TileService},
};
use anyhow::{Result, ensure};
use chrono::Utc;
use object_store::{ObjectStore, aws::AmazonS3Builder};
use std::{sync::Arc, time::Duration};

pub fn enabled() -> Result<bool> {
    Ok(radar_core::env::bool_var("LEVEL2_VIEWPORT_ENABLED", false)?)
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("{name} required"))
}

struct Parts {
    url: String,
    schema: String,
    pg: Arc<PgStore>,
    objects: Arc<dyn ObjectStore>,
    catalogue: Arc<Catalogue>,
    enabled: Vec<String>,
    namespace: String,
}

async fn parts() -> Result<Parts> {
    ensure!(enabled()?, "Level II viewport gate disabled");
    let url = required("LEVEL2_PG_URL")?;
    let schema = std::env::var("LEVEL2_PG_SCHEMA").unwrap_or_else(|_| "level2_numeric".into());
    let pool = Pool::connect(&url, &schema).await?;
    pool.initialize().await?;
    let catalogue = Arc::new(pool.catalogue().await?.unwrap_or(Catalogue::snapshot()?));
    let pg = Arc::new(PgStore::new(pool).await?);
    let endpoint = required("LEVEL2_S3_ENDPOINT")?;
    let objects: Arc<dyn ObjectStore> = Arc::new(
        AmazonS3Builder::new()
            .with_endpoint(&endpoint)
            .with_allow_http(endpoint.starts_with("http://"))
            .with_bucket_name(required("LEVEL2_S3_BUCKET")?)
            .with_region("us-east-1")
            .with_access_key_id(required("LEVEL2_S3_ACCESS_KEY")?)
            .with_secret_access_key(required("LEVEL2_S3_SECRET_KEY")?)
            .build()?,
    );
    let enabled = std::env::var("LEVEL2_ENABLED_STATIONS")
        .unwrap_or_else(|_| "KGRR".into())
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(Parts {
        url,
        schema,
        pg,
        objects,
        catalogue,
        enabled,
        namespace: required("LEVEL2_OBJECT_PREFIX")?,
    })
}

fn clock() -> Clock {
    Arc::new(Utc::now)
}

/// The read side for `radar-api`. It holds no tile renderer.
pub async fn service() -> Result<Level2Service> {
    let p = parts().await?;
    Level2Service::new(
        p.pg.clone(),
        p.pg,
        p.objects,
        p.catalogue,
        p.enabled,
        p.namespace,
        clock(),
    )
}

/// Builds ready viewports and collects expired objects until the process stops.
pub async fn publish_worker() -> Result<()> {
    ensure!(
        radar_core::env::bool_var("LEVEL2_TILE_ENABLED", false)?,
        "Level II tile gate disabled"
    );
    let p = parts().await?;
    let tiles = TileService::new(
        Pool::connect(&p.url, &p.schema).await?,
        p.objects.clone(),
        Limits::default(),
    )?;
    let publisher = Publisher {
        store: p.pg.clone(),
        index: p.pg,
        tiles: Arc::new(tiles),
        objects: p.objects,
        catalogue: p.catalogue,
        clock: clock(),
        instance: uuid::Uuid::new_v4().simple().to_string(),
    };
    let mut last_gc = std::time::Instant::now();
    loop {
        match publisher.run_once().await {
            Ok(crate::publish::Done::Idle) => tokio::time::sleep(Duration::from_millis(500)).await,
            Ok(done) => eprintln!("level2_publish={done:?}"),
            Err(e) => {
                eprintln!("level2_publish_error={e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        if last_gc.elapsed() >= Duration::from_secs(30) {
            last_gc = std::time::Instant::now();
            match publisher.collect().await {
                Ok(n) if n > 0 => eprintln!("level2_collected={n}"),
                Ok(_) => {}
                Err(e) => eprintln!("level2_collect_error={e}"),
            }
        }
    }
}
