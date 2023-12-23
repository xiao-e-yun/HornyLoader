use notify::{ReadDirectoryChangesWatcher, Watcher};
use rfd::FileDialog;
use std::{
    borrow::BorrowMut,
    collections::HashSet,
    env, fs, mem,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use crate::{list_and_choose, load, read_input};

pub fn main() -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<DevThreadMessage>();
    let mut config = DevConfig::new();

    thread::spawn(move || {
        let mut config = DevConfig::new();
        let mut file_watcher = FileWatcher::new();
        let mut dds_parser = DDSParser::new(config.from_path.get());

        let mut need_build = 0_usize;
        loop {
            let mut force_build = false;
            let mut parse_list = HashSet::new();

            //if config updated
            if let Some(event) = rx.try_recv().ok() {
                match event {
                    DevThreadMessage::Config(new_config) => {
                        dds_parser.reload(new_config.from_path.get());

                        if new_config.hot_reload.get() {
                            file_watcher.watch(new_config.from_path.get());
                        } else {
                            file_watcher.close()
                        }

                        config = new_config
                    }
                    DevThreadMessage::Update => {
                        let textures_dir = config.from_path.get().join("textures");
                        if textures_dir.is_dir() {
                            let textures = textures_dir
                                .read_dir()
                                .unwrap()
                                .filter_map(|e| e.ok().and_then(|e| Some(e.path())));
                            parse_list.extend(textures);
                        };
                        force_build = true
                    }
                    DevThreadMessage::Close => {
                        file_watcher.close();
                        return;
                    }
                }
            } else {
                //watch file
                for event in file_watcher.get() {
                    match event {
                        FileUpdateMessage::ParseDDS(path) => {
                            parse_list.insert(path);
                        }
                        FileUpdateMessage::Rebuild => need_build += 2,
                    };
                }
            }

            //
            for path in parse_list {
                dds_parser.parse(path);
            }

            //
            let hot_rebuild = {
              match need_build {
                0 => false,
                1 => {
                  need_build -= 1; 
                  println!("waiting... (waiting 3s)");
                  thread::sleep(Duration::from_secs(3));
                  true
                },
                _ => {
                  need_build -= 1;
                  println!("waiting... (waiting {}s)",3+need_build);
                  thread::sleep(Duration::from_millis(500));
                  false
                }
              }
            };
            if dds_parser.finished() && (force_build || hot_rebuild) {
                let path = config.from_path.get().clone();
                if config.name.get().is_empty() {
                    eprintln!("Miss Char name");
                    continue;
                } else if !path.join("assets").exists() {
                    eprintln!("Assets folder not found");
                    continue;
                } else if !path.join("temp").exists() {
                    eprintln!("Temp folder not found");
                    continue;
                }

                load::build_genshin_mod(&path, config.name.get(), true, String::new()).unwrap();

                let from_path = path.join("output");
                let to_path = config.to_path.get();

                //if input != output
                if path != to_path {
                    move_files(from_path, to_path, 0);
                    fn move_files(from: PathBuf, to: PathBuf, depth: usize) {
                        for file in from.read_dir().unwrap() {
                            if let Ok(file) = file {
                                let filename = file.file_name().to_string_lossy().to_string();
                                let filepath = file.path();
                                if filepath.is_dir() {
                                    let output_folder = to.join(filename);
                                    println!(
                                        "* {} -> {}",
                                        filepath.display(),
                                        output_folder.display()
                                    );
                                    if !output_folder.exists() {
                                        fs::create_dir(&output_folder).unwrap();
                                    }
                                    move_files(filepath, output_folder, depth + 1);
                                } else {
                                    let to = to.join(filename);
                                    println!(
                                        "{}* {} -> {}",
                                        "|".repeat(depth),
                                        filepath.display(),
                                        to.display()
                                    );
                                    fs::rename(&filepath, &to)
                                        .unwrap_or_else(|e| eprintln!("{}", e))
                                }
                            }
                        }
                    }
                }
            }

            //add idle time
            thread::sleep(Duration::from_millis(500));
        }
    });

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
            0 => config.name.set(),
            1 => config.from_path.set(),
            2 => config.to_path.set(),
            3 => config.parse_dds.set(),
            4 => {
                config.hot_reload.set();
                force = true;
            }
            5 => {
                tx.send(DevThreadMessage::Close).unwrap();
                break;
            }
            6 => force = true,
            _ => unreachable!(),
        };

        tx.send(DevThreadMessage::Config(config.clone())).unwrap();

        if force {
            tx.send(DevThreadMessage::Update).unwrap();
        }
    }
    Ok(())
}

enum DevThreadMessage {
    Config(DevConfig),
    Update,
    Close,
}

#[derive(PartialEq, Eq, Hash, Clone)]
enum FileUpdateMessage {
    ParseDDS(PathBuf),
    Rebuild,
}

struct FileWatcher {
    events: Arc<Mutex<HashSet<FileUpdateMessage>>>,
    inner: ReadDirectoryChangesWatcher,
    curr_ref: Arc<Mutex<PathBuf>>,
    current: Option<PathBuf>,
}

impl FileWatcher {
    fn new() -> FileWatcher {
        let events = Arc::new(Mutex::new(HashSet::new()));
        let events_copied = events.clone();
        let curr_ref = Arc::new(Mutex::new(PathBuf::new()));
        let source = curr_ref.clone();
        let watcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(e) = res {
                    let source = source.lock().unwrap().clone();
                    let root_path = PathBuf::from(source);

                    let mut events = events_copied.lock().unwrap();
                    for path in e.paths {
                        let rebuild = path.starts_with(root_path.join("assets"))
                            || path.starts_with(root_path.join("temp"));
                        let update_texture = path.starts_with(root_path.join("textures"));
                        if rebuild {
                            events.insert(FileUpdateMessage::Rebuild);
                        } else if update_texture {
                            events.insert(FileUpdateMessage::ParseDDS(path));
                        };
                    }
                }
            })
            .unwrap();
        Self {
            inner: watcher,
            current: None,
            curr_ref,
            events,
        }
    }
    fn watch(&mut self, path: PathBuf) {
        if Some(path.clone()) == self.current {
            return; //skip
        }
        self.close();
        self.inner
            .watch(path.as_path(), notify::RecursiveMode::Recursive)
            .unwrap();
        self.current = Some(path.clone());
        *self.curr_ref.lock().unwrap() = path;
    }
    fn close(&mut self) {
        if let Some(path) = &self.current {
            self.inner.unwatch(path.as_path()).unwrap();
        }
        self.current = None;
        self.get();
    }
    fn get(&self) -> HashSet<FileUpdateMessage> {
        let mut events = self.events.lock().unwrap();
        mem::replace(events.borrow_mut(), HashSet::new())
    }
}

struct DDSParser {
    path: PathBuf,
    running: Arc<Mutex<HashSet<PathBuf>>>,
}

impl DDSParser {
    fn new(path: PathBuf) -> DDSParser {
        let running = Arc::new(Mutex::new(HashSet::new()));
        DDSParser { running, path }
    }
    fn parse(&mut self, path: PathBuf) {
        if !path.is_file() {
            return;
        }

        let in_queue = self.running.lock().unwrap().insert(path.clone());
        if !in_queue {
            return;
        }

        let running = self.running.clone();
        let output_path = self.path.clone();
        thread::spawn(move || {
            if let Ok(image) = image::open(path.clone()) {
                println!("Parse dds file ({})", path.display());
                let image = image.to_rgba8();
                let dds = image_dds::dds_from_image(
                    &image,
                    image_dds::ImageFormat::BC7Srgb,
                    image_dds::Quality::Slow,
                    image_dds::Mipmaps::Disabled,
                )
                .unwrap();
                let dds_path = path.file_stem().unwrap().to_string_lossy().to_string() + ".dds";
                let mut writer = std::io::BufWriter::new(
                    std::fs::File::create(&output_path.join("assets").join(dds_path)).unwrap(),
                );
                dds.write(&mut writer).unwrap();
            }
            running.lock().unwrap().remove(&path);
        });
    }
    /// block util finished
    fn finished(&self) -> bool {
        if self.running.lock().unwrap().len() == 0 {return true;}
        while self.running.lock().unwrap().len() != 0 {
            thread::sleep(Duration::from_secs(1));
        }
        false
    }
    fn reload(&mut self, path: PathBuf) {
        if path != self.path {
            self.path = path;
        }
    }
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
impl InputOption<PathBuf> for FolderOption {
    fn set(&mut self) {
        let folder = FileDialog::new().pick_folder();
        match folder {
            Some(folder) => self.0 = folder,
            None => println!("No Folder picked"),
        }
    }

    fn get(&self) -> PathBuf {
        self.0.clone()
    }

    fn display(&self) -> String {
        self.get().to_string_lossy().to_string()
    }
}

#[derive(Debug, Clone)]
struct StringOption(String);
impl StringOption {
    fn new(string: String) -> Self {
        StringOption(string)
    }
}
impl InputOption<String> for StringOption {
    fn set(&mut self) {
        println!("waiting input");
        self.0 = read_input()
    }
    fn get(&self) -> String {
        self.0.clone()
    }

    fn display(&self) -> String {
        if self.0.is_empty() {
            "<NONE>".to_string()
        } else {
            self.get()
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
impl InputOption<bool> for BoolOption {
    fn set(&mut self) {
        self.0 = !self.0
    }
    fn get(&self) -> bool {
        self.0
    }

    fn display(&self) -> String {
        if self.0 { "Enabled" } else { "Disabled" }.to_string()
    }
}

trait InputOption<T> {
    fn get(&self) -> T;
    fn set(&mut self);
    fn display(&self) -> String;
    fn format(&self, name: &str) -> String {
        format!("{}: {}", name, self.display())
    }
}
