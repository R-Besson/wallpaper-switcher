#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Context, Result};
use display_info::DisplayInfo;
use rand::{prelude::IteratorRandom, seq::IndexedRandom};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{
	menu::{Menu, MenuItem},
	tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
	App, AppHandle, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::{fs, sync::Mutex, time};
use user_idle::UserIdle;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct Source {
	query: String,
	#[serde(default = "default_source_kind")]
	#[serde(rename = "type")]
	kind: String,
	enabled: bool,
}

fn default_source_kind() -> String {
	"unsplash".to_string()
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct Config {
	api_key: String,
	delay: String,
	theme_color: String,
	minimize_to_tray: bool,
	#[serde(default)]
	run_on_startup: bool,
	sources: Vec<Source>,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			api_key: "".into(),
			delay: "1h".into(),
			theme_color: "#FFDDDD".into(),
			minimize_to_tray: true,
			run_on_startup: false,
			sources: vec![Source {
				query: "nature".into(),
				kind: default_source_kind(),
				enabled: true,
			}],
		}
	}
}

#[derive(Deserialize, Debug)]
struct UnsplashPhoto {
	width: u32,
	height: u32,
	urls: UnsplashUrls,
}

#[derive(Deserialize, Debug)]
struct UnsplashUrls {
	raw: String,
}

#[derive(Deserialize, Debug)]
struct WallhavenResponse {
	data: Vec<WallhavenImage>,
}

#[derive(Deserialize, Debug)]
struct WallhavenImage {
	path: String,
	dimension_x: u32,
	dimension_y: u32,
}

struct AppState {
	config: Arc<Mutex<Config>>,
}

fn get_config_path() -> PathBuf {
	println!("//DEBUG// Determining config path...");
	let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
	path.push("WallpaperSwitcher/settings.json");
	println!("//DEBUG// Config path resolved to: {:?}", path);
	path
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
	println!("//DEBUG// Command 'get_config' invoked.");
	let c = state.config.lock().await;
	println!("//DEBUG// Returning config: {:?}", *c);
	Ok(c.clone())
}

#[tauri::command]
async fn save_config(
	app: AppHandle,
	config: Config,
	state: State<'_, AppState>,
) -> Result<(), String> {
	println!("//DEBUG// Command 'save_config' invoked with: {:?}", config);

	let autostart = app.autolaunch();
	if config.run_on_startup {
		println!("//DEBUG// Enabling autostart");
		let _ = autostart.enable();
	} else {
		println!("//DEBUG// Disabling autostart");
		let _ = autostart.disable();
	}

	*state.config.lock().await = config.clone();

	let path = get_config_path();
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}

	let json = serde_json::to_string_pretty(&config).map_err(|e| {
		println!("//ERROR// Failed to serialize config: {}", e);
		e.to_string()
	})?;

	std::fs::write(&path, json).map_err(|e| {
		println!("//ERROR// Failed to write config file: {}", e);
		e.to_string()
	})?;

	println!("//DEBUG// Config saved successfully to {:?}", path);
	Ok(())
}

async fn download_and_set(client: &reqwest::Client, url: &str, path: &PathBuf) -> Result<()> {
	println!("//DEBUG// Downloading image from: {}", url);
	let bytes = client.get(url).send().await?.bytes().await?;
	println!("//DEBUG// Downloaded {} bytes.", bytes.len());

	fs::write(path, &bytes).await?;
	println!("//DEBUG// Image written to disk at {:?}", path);

	wallpaper::set_from_path(path.to_str().unwrap()).map_err(|e| {
		println!("//ERROR// Failed to set wallpaper path: {}", e);
		anyhow!(e.to_string())
	})?;

	wallpaper::set_mode(wallpaper::Mode::Crop).map_err(|e| {
		println!("//ERROR// Failed to set wallpaper mode: {}", e);
		anyhow!(e.to_string())
	})?;

	println!("//DEBUG// Wallpaper updated successfully.");
	Ok(())
}

async fn fetch_unsplash(
	client: &reqwest::Client,
	keywords: &str,
	api_key: &str,
	min_w: u32,
	min_h: u32,
) -> Result<String> {
	println!("//DEBUG// Fetching Unsplash: {}", keywords);
	let auth_header = if api_key.starts_with("Client-ID") {
		api_key.to_string()
	} else {
		format!("Client-ID {}", api_key)
	};

	let response = client
		.get("https://api.unsplash.com/photos/random")
		.query(&[("query", keywords), ("orientation", "landscape")])
		.header("Authorization", auth_header)
		.send()
		.await?;

	if !response.status().is_success() {
		return Err(anyhow!("Unsplash API error: {}", response.status()));
	}

	let photo: UnsplashPhoto = response.json().await?;
	if photo.width >= min_w && photo.height >= min_h {
		Ok(photo.urls.raw)
	} else {
		Err(anyhow!("Image too small"))
	}
}

async fn fetch_wallhaven(
	client: &reqwest::Client,
	query: &str,
	min_w: u32,
	min_h: u32,
) -> Result<String> {
	println!("//DEBUG// Fetching Wallhaven: {}", query);

	let response = client
		.get("https://wallhaven.cc/api/v1/search")
		.query(&[
			("q", query),
			("sorting", "random"),
			("atleast", &format!("{}x{}", min_w, min_h)),
		])
		.send()
		.await?;

	if !response.status().is_success() {
		return Err(anyhow!("Wallhaven API error: {}", response.status()));
	}

	let json: WallhavenResponse = response.json().await?;

	json.data
		.iter()
		.filter(|img| img.dimension_x >= min_w && img.dimension_y >= min_h)
		.choose(&mut rand::rng())
		.map(|img| img.path.clone())
		.ok_or_else(|| anyhow!("No images found on Wallhaven"))
}

async fn update_wallpaper(
	client: &reqwest::Client,
	config: Config,
	image_path: PathBuf,
) -> Result<()> {
	println!("//DEBUG// Update task started.");

	let (max_w, max_h) = DisplayInfo::all()
		.unwrap_or_default()
		.iter()
		.map(|d| (d.width, d.height))
		.max()
		.unwrap_or((1920, 1080));

	let enabled_sources: Vec<_> = config.sources.iter().filter(|s| s.enabled).collect();

	if enabled_sources.is_empty() {
		return Err(anyhow!("No sources enabled"));
	}

	for _ in 0..3 {
		let source = enabled_sources.choose(&mut rand::rng()).context("Empty")?;

		let result = match source.kind.as_str() {
			"wallhaven" => fetch_wallhaven(client, &source.query, max_w, max_h).await,
			_ => fetch_unsplash(client, &source.query, &config.api_key, max_w, max_h).await,
		};

		match result {
			Ok(url) => {
				download_and_set(client, &url, &image_path).await?;
				return Ok(());
			}
			Err(e) => {
				eprintln!("//ERROR// Failed to fetch from {}: {}", source.kind, e);
				time::sleep(Duration::from_secs(2)).await;
			}
		}
	}

	Err(anyhow!("Failed to update wallpaper after retries"))
}

async fn run_service(shared_config: Arc<Mutex<Config>>) {
	println!("//DEBUG// Background service initialized.");
	let client = reqwest::Client::new();
	let image_path = dirs::cache_dir().unwrap_or_default().join("current_wp.jpg");
	println!("//DEBUG// Image cache path: {:?}", image_path);

	loop {
		println!("//DEBUG// Service loop tick.");
		let config = shared_config.lock().await.clone();

		let has_enabled_sources = config.sources.iter().any(|s| s.enabled);
		if has_enabled_sources {
			let idle_seconds = UserIdle::get_time().map(|i| i.as_seconds()).unwrap_or(0);

			println!("//DEBUG// User idle time: {} seconds.", idle_seconds);

			if idle_seconds < 600 {
				println!("//DEBUG// User active (idle < 600s). Attempting update...");
				if let Err(e) = update_wallpaper(&client, config.clone(), image_path.clone()).await
				{
					eprintln!("//ERROR// Wallpaper update failed: {}", e);
				}
			} else {
				println!("//DEBUG// User idle too long (> 600s). Skipping update.");
			}
		} else {
			println!("//DEBUG// No sources enabled (or API key check removed). Skipping update.");
		}

		let delay = humantime::parse_duration(&config.delay).unwrap_or(Duration::from_secs(3600));

		println!("//DEBUG// Sleeping for {:?} before next update.", delay);
		time::sleep(delay).await;
	}
}

fn setup_tray(app: &App) -> Result<()> {
	let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
	let show_i = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
	let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

	TrayIconBuilder::new()
		.icon(app.default_window_icon().unwrap().clone())
		.menu(&menu)
		.show_menu_on_left_click(false)
		.on_menu_event(|app, event| {
			println!("//DEBUG// Menu event: {}", event.id().as_ref());
			match event.id().as_ref() {
				"quit" => {
					println!("//DEBUG// Quitting application via menu.");
					app.exit(0);
				}
				"show" => {
					println!("//DEBUG// Showing main window via menu.");
					if let Some(window) = app.get_webview_window("main") {
						let _ = window.show();
						let _ = window.unminimize();
						let _ = window.set_focus();
					}
				}
				_ => {}
			}
		})
		.on_tray_icon_event(|tray, event| {
			if let TrayIconEvent::Click {
				button: MouseButton::Left,
				..
			} = event
			{
				println!("//DEBUG// Left click on Tray Icon.");
				if let Some(window) = tray.app_handle().get_webview_window("main") {
					let _ = window.show();
					let _ = window.unminimize();
					let _ = window.set_focus();
				}
			}
		})
		.build(app)?;

	Ok(())
}

fn main() {
	println!("//DEBUG// Application starting...");
	let path = get_config_path();

	let initial_config = if path.exists() {
		println!("//DEBUG// Loading existing config file.");
		std::fs::read_to_string(path)
			.ok()
			.and_then(|s| serde_json::from_str(&s).ok())
			.unwrap_or_else(|| {
				println!("//ERROR// Failed to parse config, using default");
				Config::default()
			})
	} else {
		println!("//DEBUG// No config file found. Using default.");
		Config::default()
	};

	println!("//DEBUG// Initial config state: {:?}", initial_config);
	let shared_config = Arc::new(Mutex::new(initial_config));

	let bg_config = shared_config.clone();
	tauri::async_runtime::spawn(async move {
		println!("//DEBUG// Spawning background thread...");
		run_service(bg_config).await;
	});

	println!("//DEBUG// Building Tauri application...");
	tauri::Builder::default()
		.plugin(tauri_plugin_opener::init())
		.plugin(tauri_plugin_autostart::init(
			MacosLauncher::LaunchAgent,
			None,
		))
		.manage(AppState {
			config: shared_config.clone(),
		})
		.setup(|app| {
			println!("//DEBUG// Tauri setup callback.");
			setup_tray(app)?;
			Ok(())
		})
		.on_window_event(move |window, event| {
			if let WindowEvent::CloseRequested { api, .. } = event {
				println!("//DEBUG// Window CloseRequested event received.");
				let config = shared_config.blocking_lock();
				if config.minimize_to_tray {
					println!("//DEBUG// Hiding window to tray (minimize_to_tray=true).");
					window.hide().unwrap();
					api.prevent_close();
				} else {
					println!("//DEBUG// Closing window (minimize_to_tray=false).");
				}
			}
		})
		.invoke_handler(tauri::generate_handler![get_config, save_config])
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}
