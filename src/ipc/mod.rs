// prism-runtime/src/ipc/mod.rs
//
// Generic IPC bridge — connects to any host application via Windows named pipes.
// The protocol is JSON-based: {ns, cmd, args} → {ok, data, error}.
// The pipe name and polling behaviour are fully configurable.
//
// IPC is generic — OpenRender passes through whatever JSON the JS runtime sends.
// OpenDesktop-specific logic (data demands, heartbeats) lives in opendesktop.js.

pub mod client;
pub mod protocol;
pub mod opendesktop;

