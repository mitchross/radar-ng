//! radar-tiler: the Rust port of radar-ng's Web Mercator tile renderer (spec-tiler.md).
//!
//! The production entry point is [`render_frame_palettes`]. It renders one source grid into
//! one atomically published, indexed-PNG pyramid per palette, and never adopts a tree that
//! lacks a completion marker. Renderer identity is `"indexed-rs1"`; see README.md for the
//! deviations from the Python implementation.

pub mod cache;
pub mod classify;
pub mod env;
pub mod error;
pub mod geometry;
pub mod identity;
pub mod marker;
pub mod palettes;
pub mod png;
pub mod publish;
pub mod render;
pub mod sample;

pub use cache::{GeometryCacheStats, clear_tile_geometry_cache, tile_geometry_cache_stats};
pub use classify::{ClassModel, PaletteLut, build_categorical_class_model, build_class_model};
pub use env::{TileRenderer, resolve_tile_renderer, tile_renderer_for_role};
pub use error::{Result, TilerError};
pub use identity::{FrameKind, PyramidIdentity, RENDERER, frame_identity_is_compatible};
pub use marker::{PyramidInspection, complete_pyramid_identity, is_complete_pyramid};
pub use palettes::{ColorTable, load_palette, palette_names};
pub use png::{DecodedIndexed, decode_indexed, encode_indexed, replace_palette};
pub use publish::{MultiPaletteRenderResult, PalettePublishOutcome, PublishStatus};
pub use radar_geo::{Crs, Grid, GridGeo};
pub use render::{FrameOptions, Overview, render_frame_palettes};
