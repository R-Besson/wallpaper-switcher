#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, Context, Result};
use display_info::DisplayInfo;
use rand::{prelude::IteratorRandom, seq::IndexedRandom};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{
	menu::{Menu, MenuItem},
	tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
	App, AppHandle, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::{fs, sync::Mutex, sync::Notify, time};
use user_idle::UserIdle;

fn default_source_kind() -> String {
	"unsplash".to_string()
}

fn default_weight() -> u32 {
	10
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct Source {
	query: String,
	#[serde(default = "default_source_kind")]
	#[serde(rename = "type")]
	kind: String,
	enabled: bool,
	#[serde(default = "default_weight")]
	weight: u32,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
struct Config {
	unsplash_api_key: String,
	delay: String,
	theme_color: String,
	minimize_to_tray: bool,
	#[serde(default)]
	run_on_startup: bool,
	sources: Vec<Source>,
	#[serde(default)]
	blacklist: Vec<String>,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			unsplash_api_key: "".into(),
			delay: "1h".into(),
			theme_color: "#FFDDDD".into(),
			minimize_to_tray: true,
			run_on_startup: false,
			sources: vec![Source {
				query: "nature".into(),
				kind: default_source_kind(),
				enabled: true,
				weight: 10,
			}],
			blacklist: vec![],
		}
	}
}

#[derive(Clone, Debug)]
struct CurrentWallpaper {
	source_index: usize,
	url: String,
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
	current_wallpaper: Arc<Mutex<Option<CurrentWallpaper>>>,
	trigger: Arc<Notify>,
}

fn get_config_path() -> PathBuf {
	let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
	path.push("WallpaperSwitcher/settings.json");
	path
}

fn persist_config(config: &Config) -> Result<()> {
	let path = get_config_path();
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	let json = serde_json::to_string_pretty(config)?;
	std::fs::write(&path, json)?;
	Ok(())
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
	let c = state.config.lock().await;
	Ok(c.clone())
}

#[tauri::command]
async fn save_config(
	app: AppHandle,
	config: Config,
	state: State<'_, AppState>,
) -> Result<(), String> {
	println!("//DEBUG// Command 'save_config' invoked.");

	let autostart = app.autolaunch();
	if config.run_on_startup {
		let _ = autostart.enable();
	} else {
		let _ = autostart.disable();
	}

	*state.config.lock().await = config.clone();

	persist_config(&config).map_err(|e| e.to_string())?;

	state.trigger.notify_one();

	Ok(())
}

async fn download_and_set(client: &reqwest::Client, url: &str, path: &PathBuf) -> Result<()> {
	println!("//DEBUG// Downloading image from: {}", url);
	let bytes = client.get(url).send().await?.bytes().await?;
	fs::write(path, &bytes).await?;

	wallpaper::set_from_path(path.to_str().unwrap()).map_err(|e| anyhow!(e.to_string()))?;
	wallpaper::set_mode(wallpaper::Mode::Crop).map_err(|e| anyhow!(e.to_string()))?;
	Ok(())
}

async fn fetch_unsplash(
	client: &reqwest::Client,
	keywords: &str,
	unsplash_api_key: &str,
	min_w: u32,
	min_h: u32,
) -> Result<String> {
	let auth_header = if unsplash_api_key.starts_with("Client-ID") {
		unsplash_api_key.to_string()
	} else {
		format!("Client-ID {}", unsplash_api_key)
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
) -> Result<(String, usize)> {
	println!("//DEBUG// Update task started.");

	let (max_w, max_h) = DisplayInfo::all()
		.unwrap_or_default()
		.iter()
		.map(|d| (d.width, d.height))
		.max()
		.unwrap_or((1920, 1080));

	let mut weighted_indices = Vec::new();
	for (index, source) in config.sources.iter().enumerate() {
		if source.enabled {
			for _ in 0..source.weight.max(1) {
				weighted_indices.push(index);
			}
		}
	}

	if weighted_indices.is_empty() {
		return Err(anyhow!("No sources enabled"));
	}

	let blacklist_set: HashSet<&String> = config.blacklist.iter().collect();

	for mut _i in 0..5 {
		let source_idx = *weighted_indices.choose(&mut rand::rng()).context("Empty")?;
		let source = &config.sources[source_idx];

		let result = match source.kind.as_str() {
			"wallhaven" => fetch_wallhaven(client, &source.query, max_w, max_h).await,
			_ => fetch_unsplash(client, &source.query, &config.unsplash_api_key, max_w, max_h).await,
		};

		match result {
			Ok(url) => {
				if blacklist_set.contains(&url) {
					println!("//DEBUG// Image is blacklisted, skipping: {}", url);
					_i-=1;
					continue;
				}
				download_and_set(client, &url, &image_path).await?;
				return Ok((url, source_idx));
			}
			Err(e) => {
				eprintln!("//ERROR// Failed to fetch: {}", e);
				time::sleep(Duration::from_secs(1)).await;
			}
		}
	}

	Err(anyhow!("Failed to update wallpaper after retries"))
}

async fn run_service(
	shared_config: Arc<Mutex<Config>>,
	current_wallpaper_state: Arc<Mutex<Option<CurrentWallpaper>>>,
	trigger: Arc<Notify>,
) {
	let client = reqwest::Client::new();
	let image_path = dirs::cache_dir().unwrap_or_default().join("current_wallpaper.jpg");

	time::sleep(Duration::from_secs(2)).await;

	loop {
		println!("//DEBUG// Loop tick.");
		let config = shared_config.lock().await.clone();
		let has_enabled_sources = config.sources.iter().any(|s| s.enabled);

		let duration =
			humantime::parse_duration(&config.delay).unwrap_or(Duration::from_secs(3600));

		if has_enabled_sources {
			let idle_seconds = UserIdle::get_time().map(|i| i.as_seconds()).unwrap_or(0);

			if idle_seconds < 600 {
				println!("//DEBUG// Attempting update...");
				match update_wallpaper(&client, config.clone(), image_path.clone()).await {
					Ok((url, index)) => {
						let mut curr = current_wallpaper_state.lock().await;
						*curr = Some(CurrentWallpaper {
							source_index: index,
							url,
						});
					}
					Err(e) => eprintln!("//ERROR// Wallpaper update failed: {}", e),
				}
			} else {
				println!("//DEBUG// User idle too long. Skipping update.");
			}
		}

		println!(
			"//DEBUG// Sleeping for {:?} or waiting for trigger.",
			duration
		);

		tokio::select! {
			_ = time::sleep(duration) => {
			}
			_ = trigger.notified() => {
				println!("//DEBUG// Trigger received. Resetting timer / Skipping.");
			}
		}
	}
}

fn setup_tray(
	app: &App,
	state_trigger: Arc<Notify>,
	shared_config: Arc<Mutex<Config>>,
	current_wallpaper: Arc<Mutex<Option<CurrentWallpaper>>>,
) -> Result<()> {
	let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
	let show_i = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;

	let next_i = MenuItem::with_id(app, "next", "Next Wallpaper", true, None::<&str>)?;
	let more_i = MenuItem::with_id(app, "more", "Show more of this", true, None::<&str>)?;
	let less_i = MenuItem::with_id(app, "less", "Show less of this", true, None::<&str>)?;
	let never_i = MenuItem::with_id(app, "never", "Never show this wallpaper again", true, None::<&str>)?;

	let menu = Menu::with_items(app, &[&next_i, &more_i, &less_i, &never_i, &show_i, &quit_i])?;

	TrayIconBuilder::new()
		.icon(app.default_window_icon().unwrap().clone())
		.menu(&menu)
		.show_menu_on_left_click(false)
		.on_menu_event(move |app, event| {
			let id = event.id().as_ref();
			match id {
				"quit" => app.exit(0),
				"show" => {
					if let Some(window) = app.get_webview_window("main") {
						let _ = window.show();
						let _ = window.unminimize();
						let _ = window.set_focus();
					}
				}
				"next" => {
					state_trigger.notify_one();
				}
				"more" => {
					let config_ptr = shared_config.clone();
					let wp_ptr = current_wallpaper.clone();

					tauri::async_runtime::spawn(async move {
						let current_wallpaper_opt = wp_ptr.lock().await;
						if let Some(ref current_wallpaper) = *current_wallpaper_opt {
							let mut conf = config_ptr.lock().await;
							if let Some(source) = conf.sources.get_mut(current_wallpaper.source_index) {
								source.weight += 1;
								println!(
									"//DEBUG// Increased weight for source {} to {}",
									current_wallpaper.source_index, source.weight
								);
								if let Err(e) = persist_config(&conf) {
									eprintln!("//ERROR// Failed to save config: {}", e);
								}
							}
						}
					});
				}
				"less" => {
					let config_ptr = shared_config.clone();
					let wp_ptr = current_wallpaper.clone();

					tauri::async_runtime::spawn(async move {
						let current_wallpaper_opt = wp_ptr.lock().await;
						if let Some(ref current_wallpaper) = *current_wallpaper_opt {
							let mut conf = config_ptr.lock().await;
							if let Some(source) = conf.sources.get_mut(current_wallpaper.source_index) {
								source.weight = std::cmp::max(1, source.weight-1);
								println!(
									"//DEBUG// Decreased weight for source {} to {}",
									current_wallpaper.source_index, source.weight
								);
								if let Err(e) = persist_config(&conf) {
									eprintln!("//ERROR// Failed to save config: {}", e);
								}
							}
						}
					});
				}
				"never" => {
					let config_ptr = shared_config.clone();
					let wp_ptr = current_wallpaper.clone();
					let trigger_ptr = state_trigger.clone();

					tauri::async_runtime::spawn(async move {
						let current_opt = wp_ptr.lock().await;
						if let Some(ref current_wallpaper) = *current_opt {
							let mut conf = config_ptr.lock().await;
							println!("//DEBUG// Blacklisting: {}", current_wallpaper.url);
							conf.blacklist.push(current_wallpaper.url.clone());

							if let Err(e) = persist_config(&conf) {
								eprintln!("//ERROR// Failed to save config: {}", e);
							} else {
								trigger_ptr.notify_one();
							}
						}
					});
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
		std::fs::read_to_string(path)
			.ok()
			.and_then(|s| serde_json::from_str(&s).ok())
			.unwrap_or_else(Config::default)
	} else {
		Config::default()
	};

	let shared_config = Arc::new(Mutex::new(initial_config));
	let current_wallpaper = Arc::new(Mutex::new(None));
	let trigger = Arc::new(Notify::new());

	let bg_config = shared_config.clone();
	let bg_curr = current_wallpaper.clone();
	let bg_trigger = trigger.clone();

	tauri::async_runtime::spawn(async move {
		run_service(bg_config, bg_curr, bg_trigger).await;
	});

	tauri::Builder::default()
		.plugin(tauri_plugin_opener::init())
		.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
			if let Some(window) = app.get_webview_window("main") {
				let _ = window.show();
				let _ = window.unminimize();
				let _ = window.set_focus();
			}
		}))
		.plugin(tauri_plugin_autostart::init(
			MacosLauncher::LaunchAgent,
			Some(vec!["--minimized"]),
		))
		.manage(AppState {
			config: shared_config.clone(),
			current_wallpaper: current_wallpaper.clone(),
			trigger: trigger.clone(),
		})
		.setup(move |app| {
			setup_tray(
				app,
				trigger.clone(),
				shared_config.clone(),
				current_wallpaper.clone(),
			)?;

			let config = shared_config.blocking_lock();
            let autostart = app.autolaunch();
            if config.run_on_startup {
                println!("//DEBUG// Ensuring autostart is enabled on launch.");
                let _ = autostart.enable();
            } else {
                let _ = autostart.disable();
            }
            drop(config);

			let args: Vec<String> = std::env::args().collect();
			if !args.contains(&"--minimized".to_string()) {
				if let Some(window) = app.get_webview_window("main") {
					let _ = window.show();
				}
			}
			Ok(())
		})
		.on_window_event(move |window, event| {
			if let WindowEvent::CloseRequested { api, .. } = event {
				window.hide().unwrap();
				api.prevent_close();
			}
		})
		.invoke_handler(tauri::generate_handler![get_config, save_config])
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}
