***
# Wallpaper Switcher

A modern desktop app that fetches high-res wallpapers from Unsplash based on groups of keywords at random. Built with **Rust** and **Tauri** for minimal resource usage and max performance. Mainly made for OLED monitors and TV's but anybody can use it.

## Prerequisites

1.  **Rust**: [https://rust-lang.org/tools/install/](https://rust-lang.org/tools/install/)
2.  **System Dependencies**:
    *   *Windows:* "C++ build tools" (via Visual Studio Build Tools).
    *   *Linux:* `sudo apt install libwebkit2gtk-4.0-dev build-essential curl wget libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
    *   *macOS:* Xcode Command Line Tools.

## Building

1.  **Clone the repo:**
    ```bash
    git clone https://github.com/R-Besson/wallpaper-switcher.git
    cd wallpaper-switcher
    ```

2.  **Install the Tauri CLI**:
    ```bash
    cargo install tauri-cli
    ```

3.  **Run in Development Mode:**
    This will compile the app and open the window with dynamic reloading upon edits.
    ```bash
    cargo tauri dev
    ```

4.  **Build for Production:**
    This creates an optimized binary and installer (EXE, DMG, or DEB depending on your OS).
    ```bash
    cargo tauri build
    ```

## Usage

### 1. Get an API Key
1.  [Make an Unsplash account](https://unsplash.com/).
2.  [Create a new Application](https://unsplash.com/oauth/applications).
3.  Copy the **Access Key** and paste it in the settings.

## Future Plans
- [x] Start-Up App
- [x] System Tray
- [x] **User Interface**
- [ ] Other sources
- [ ] Local file/folder support
- [ ] Different wallpapers for each display
- [ ] Mosaic/Collage mode