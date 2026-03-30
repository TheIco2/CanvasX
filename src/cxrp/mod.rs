// openrender-runtime/src/cxrp/mod.rs
//
// CXRP — OpenRender Runtime Package
//
// A .cxrp file bundles multiple .cxrd documents and their associated assets
// into a single distributable archive. The format is a ZIP archive with a
// well-defined internal structure:
//
//   manifest.json         — package metadata & file inventory
//   documents/            — .cxrd files
//   assets/
//     images/             — shared images  
//     fonts/              — shared fonts
//     audio/              — shared audio
//     video/              — shared video
//   libraries/            — referenced .cxrl files
//
// Any host application can load .cxrp packages to render OpenRender content.

pub mod manifest;
pub mod loader;
pub mod builder;
