use notify::Watcher;
use rfd::FileDialog;
use std::{
    env,
    fs::rename,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use crate::{list_and_choose, load, read_input};

pub fn main() -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<DevConfig>();
    let mut config = DevConfig::new();

    thread::spawn(move || {
        let config = Arc::new(Mutex::new(DevConfig::new()));
        let watcher_config = config.clone();
        let changed = Arc::new(Mutex::new(false));
        let file_changed = changed.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(e) = res {
                    let mut changed = false;
                    let config = watcher_config.lock().unwrap();
                    let root_path = config.from_path.0.clone();
                    let hot_reload = config.hot_reload.0.clone();
                    let temp_dir = root_path.join("temp");
                    let assets_dir = root_path.join("assets");
                    let textures_dir = root_path.join("textures");
                    for path in e.paths {
                        changed = changed
                            || path.starts_with(&temp_dir)
                            || (path.starts_with(&assets_dir)
                                && !(hot_reload && path.ends_with(".dds")))
                            || path.starts_with(&textures_dir);
                    }

                    let mut global_changed = file_changed.lock().unwrap();
                    if changed && !*global_changed {
                        println!("File changed (Waiting 1s)");
                        thread::sleep(Duration::from_secs(1));
                        *global_changed = changed;
                    }
                }
            })
            .unwrap();

        loop {
            if let Some(new_config) = rx.try_recv().ok() {
                if config.lock().unwrap().hot_reload.0 {
                    watcher
                        .unwatch(config.lock().unwrap().from_path.0.as_path())
                        .unwrap();
                }
                if new_config.hot_reload.0 {
                    watcher
                        .watch(
                            new_config.from_path.0.as_path(),
                            notify::RecursiveMode::Recursive,
                        )
                        .unwrap();
                }
                *config.lock().unwrap() = new_config;
                println!("Config Changed");
                *changed.lock().unwrap() = true;
            }

            thread::sleep(Duration::from_secs(1));
            if *changed.lock().unwrap() {
                let config = config.lock().unwrap().clone();
                let path = config.from_path.0;

                if config.name.0.is_empty() {
                    continue;
                } else if !path.join("assets").exists() {
                    eprintln!("Assets folder Not found");
                    continue;
                } else if !path.join("temp").exists() {
                    eprintln!("Temp folder not found");
                    continue;
                } else if path.join("textures").exists() {
                    println!("found textures, auto parse to dds");
                    for file in path.join("textures").read_dir().unwrap() {
                        let file_path = file.unwrap().path();
                        // accept PNG
                        if !file_path.is_file() || file_path.extension().unwrap() != "png" {
                            continue;
                        }
                        println!("parse {}", file_path.display());
                        let image = image::open(file_path.clone()).unwrap().to_rgba8();
                        let format = image_dds::ImageFormat::BC7Srgb;
                        let dds = image_dds::dds_from_image(
                            &image,
                            format,
                            image_dds::Quality::Fast,
                            image_dds::Mipmaps::Disabled,
                        )
                        .unwrap();
                        let mut writer = std::io::BufWriter::new(
                            std::fs::File::create(&path.join("assets").join(
                                file_path.file_stem().unwrap().to_string_lossy().to_string()
                                    + ".dds",
                            ))
                            .unwrap(),
                        );
                        dds.write(&mut writer).unwrap();
                    }
                }

                println!("Build genshin mod");
                load::build_genshin_mod(&path, config.name.0, true, String::new())
                    .unwrap_or_else(|e| eprintln!("{}", e));

                let from_path = path.join("output");
                let to_path = config.to_path.0;

                if path != to_path {
                    println!("{} -> {}", from_path.display(), to_path.display());
                    for file in from_path.read_dir().unwrap() {
                        let path = file.unwrap().path();
                        rename(path.clone(), to_path.join(path.file_name().unwrap()))
                            .unwrap_or_else(|e| eprintln!("{}", e))
                    }
                }

                *changed.lock().unwrap() = false;
            }
        }
    });

    tx.send(config.clone()).unwrap();
    loop {
        let action = list_and_choose(
            "Settings",
            vec![
                config.name.format("Character Name"),
                config.from_path.format("Mod Path (From)"),
                config.to_path.format("Install Path (To)"),
                config.parse_dds.format("Auto Parse to DDS"),
                config.hot_reload.format("Hot Reload"),
                "Exit".to_string(),
                "Update".to_string(),
            ],
            false,
        );
        let mut force = false;
        match action {
            0 => config.name.set_value(),
            1 => config.from_path.set_value(),
            2 => config.to_path.set_value(),
            3 => config.parse_dds.set_value(),
            4 => {
                config.hot_reload.set_value();
                force = true;
            }
            5 => break,
            6 => force = true,
            _ => unreachable!(),
        };
        if config.hot_reload.0 || force {
            println!("Updateing");
            tx.send(config.clone()).unwrap();
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct DevConfig {
    name: StringOption,
    to_path: FolderOption,
    from_path: FolderOption,
    parse_dds: BoolOption,
    hot_reload: BoolOption,
}

impl DevConfig {
    fn new() -> DevConfig {
        let current_path = env::current_dir().unwrap();
        DevConfig {
            name: StringOption::new(String::new()),
            to_path: FolderOption::new(current_path.clone()),
            from_path: FolderOption::new(current_path.clone()),
            parse_dds: BoolOption::new(true),
            hot_reload: BoolOption::new(false),
        }
    }
}

#[derive(Debug, Clone)]
struct FolderOption(PathBuf);
impl FolderOption {
    fn new(path: PathBuf) -> Self {
        FolderOption(path)
    }
}
impl Option for FolderOption {
    fn set_value(&mut self) {
        let folder = FileDialog::new().pick_folder();
        match folder {
            Some(folder) => self.0 = folder,
            None => println!("No Folder picked"),
        }
    }

    fn display(&self) -> String {
        self.0.to_string_lossy().to_string()
    }
}

#[derive(Debug, Clone)]
struct StringOption(String);
impl StringOption {
    fn new(string: String) -> Self {
        StringOption(string)
    }
}
impl Option for StringOption {
    fn set_value(&mut self) {
        println!("waiting input");
        self.0 = read_input()
    }

    fn display(&self) -> String {
        if self.0.is_empty() {
            "<NONE>".to_string()
        } else {
            self.0.clone()
        }
    }
}

#[derive(Debug, Clone)]
struct BoolOption(bool);
impl BoolOption {
    fn new(bool: bool) -> Self {
        BoolOption(bool)
    }
}
impl Option for BoolOption {
    fn set_value(&mut self) {
        self.0 = !self.0
    }

    fn display(&self) -> String {
        if self.0 { "Enabled" } else { "Disabled" }.to_string()
    }
}

trait Option {
    fn set_value(&mut self);
    fn display(&self) -> String;
    fn format(&self, name: &str) -> String {
        format!("{}: {}", name, self.display())
    }
}
