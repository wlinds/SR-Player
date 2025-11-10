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

pub mod api; // Exposes core/api.rs - API client for Sveriges Radio
pub mod gapless_send_safe;
pub mod gapless_streaming; // Exposes core/gapless_streaming.rs - Gapless player with Symphonia
pub mod models; // Exposes core/models.rs - Data structures for API responses
pub mod podcast; // Exposes core/podcast.rs - Podcast handling utilities
pub mod utils; // Exposes core/utils.rs - Utility functions for image and date handling
