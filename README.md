# <img src="src/assets/icons/icon.png" alt="alt text" width="32" height="32" /> SR Player - Sveriges Radio Desktop Player


A super fast and lightweight macOS (soon cross-platform) desktop app for streaming Sveriges Radio live channels, built with Rust, Slint, Symphonia and Rodio.


![alt text](images/sr-player-025-11-04.png)

## Live Features

- Stream all Sveriges Radio live channels
- Gapless and super fast audio playback
- Real-time "Now Playing" information
- Quick channel browsing
- Native macOS .app bundle (~5MB)

## Current Status: Milestone 3 Complete

The code is extensively commented and easy for new Rust developers to get into, possibly comming from Python or JavaScript.

**Working:**
- Gapless streaming using Symphonia decoder
- Fetch and display all Sveriges Radio channels from API
- Real-time "Now Playing" program information from SR API
- Native macOS .app
- Optimized release builds (~5MB total size)
- Thread-safe async streaming architecture

## **8. Milestone Plan**

| Stage | Description |
|--------|--------------|
| M1 | Minimum UI & single stream playback
| M2 | Full channel list browsing
| M3 | Gapless streaming, macOS build
| M4 | Podcast browsing & playback, Linux & Windows build 
| M5 | Polished UI & improved packaging

## Quick Start

### Development Mode
```bash
# Build and run
cargo run
```

[Syntax highlighting in VSCode for Slint](https://marketplace.visualstudio.com/items?itemName=Slint.slint)

### Production Build (macOS App)

```bash
# Install cargo-bundle (first time only)
cargo install cargo-bundle

# Build optimized .app bundle
cargo bundle --release

# Result: target/release/bundle/osx/SR Player.app
```

## Binary Size Optimization

The release build is configured for minimal size in `Cargo.toml`:

```toml
[profile.release]
strip = true          # Strip debug symbols (~60% size reduction)
opt-level = "z"       # Optimize for size
lto = true            # Link-time optimization
codegen-units = 1     # Better optimization
panic = "abort"       # Remove unwinding machinery
```

**Size breakdown:**
- Before optimization: ~15MB (14MB binary + 1MB icon)
- After optimization: ~3-5MB total
- Comparison: Electron app (80-150MB)

## Technical Architecture

**Gapless Streaming:**
- HTTP Stream → Custom MediaSource → Symphonia Decoder → Rodio Sink
- Dedicated audio thread for thread-safe non-Send types
- Async HTTP streaming with tokio
- Eliminates gaps between audio chunks

---


### **License**
MPL-2.0 for open development compatibility.  
Uses Sveriges Radio API under Creative Commons Attribution stipulations.