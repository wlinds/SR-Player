// This file declares all the submodules in the 'core' module
//
// MODULE SYSTEM IN RUST (compared to Python and JavaScript):
//
// Python equivalent:
//   # In core/__init__.py
//   from . import api
//   from . import audio
//   from . import models
//
// JavaScript/Node.js equivalent:
//   // In core/index.js
//   export * from './api.js';
//   export * from './audio.js';
//   export * from './models.js';
//
// Rust uses:
//   pub mod api;     // Makes the api module public
//
// The 'pub' keyword is like 'export' in JavaScript or public classes in Python.
// Without 'pub', the module would be private to the core module only.

pub mod api;
pub mod channel_pool;
pub mod dvr_buffer;
pub mod episode_cache;
pub mod favorites;
pub mod file_player;
pub mod file_player_send_safe;
pub mod gapless_send_safe;
pub mod gapless_streaming;
pub mod models;
pub mod podcast;
pub mod seek_handler;
pub mod settings;
pub mod utils;
