use serde::{Deserialize, Serialize};
use sevenz_rust::{Archive, BlockDecoder};
use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
};

use crate::{list_and_choose, BREAK_LINE};

const MOD_CONFIG: &str = "config.hl.json";

pub fn main() -> Result<(), String> {
    //load MOD_CONFIG
    let path = Path::new(MOD_CONFIG);
    let json = if path.exists() {
        println!("Loading `{}`", MOD_CONFIG);
        fs::read_to_string(path).map_err(|v| v.to_string())
    } else {
        Err(format!("`{}` Not Found", MOD_CONFIG))
    }?;

    let config: ModConfig = serde_json::from_str(&*json).unwrap();

    println!("{}", BREAK_LINE);
    println!("Name: {}", config.name);
    println!("Options: {}", config.options.len());
    println!("{}", BREAK_LINE);

    let choose = list_and_choose("Operate", vec!["Choose Variants", "Exit"], true);
    match choose {
        0 => choose_variants(config),
        1 => Ok(()),
        _ => unreachable!(),
    }
}

fn choose_variants(config: ModConfig) -> Result<(), String> {
    let mut chooses = vec![0_usize; config.options.len()];
    let exit_index = config.options.len();
    loop {
        let choose = list_and_choose("", get_list(&config, &chooses), false);

        if choose == exit_index {
            break;
        } else if choose == exit_index + 1 {
            // finished_index as exit_index + 1
            let mut id = String::new();
            for value in &chooses {
                //break on over 16
                id += &format!("{:x}", value);
            }
            println!("Loading {}", id);

            let path = env::current_dir().unwrap();
            let temp = path.join("temp");
            extract(&path, &temp, &id);
            build_genshin_mod(path.as_path(), config.name.clone(), true, id)?;
            fs::remove_dir_all(temp).unwrap();
        } else if choose < exit_index {
            //variants
            let (name, list) = config.options[choose].clone();
            let variant = list_and_choose(format!("Changing `{}` Variants", name), list, false);
            chooses[choose] = variant
        } else if choose > exit_index + 1 {
            unreachable!()
        }
    }

    fn get_list(config: &ModConfig, chooses: &Vec<usize>) -> Vec<String> {
        let mut list: Vec<String> = config
            .options
            .iter()
            .enumerate()
            .map(|(index, (name, options))| format!("{}: {}", name, options[chooses[index]]))
            .collect();
        list.extend(["Exit".to_string(), "Finished".to_string()]);
        list
    }

    Ok(())
}

fn extract(path: &Path, to: &Path, target: &str) {
    let zip_path = path.join("package.7z");
    let mut zip = File::open(zip_path).unwrap();
    let len = zip.metadata().unwrap().len();
    let arch = Archive::read(&mut zip, len, &[]).unwrap();
    let folder_count = arch.folders.len();

    println!("=Extract={}", BREAK_LINE);
    for folder_index in 0..folder_count {
        let forder_dec = BlockDecoder::new(folder_index, &arch, &[], &mut zip);
        forder_dec
            .for_each_entries(&mut |entry, reader| {
                let name = entry.name();
                if name.starts_with(&format!("{}/", target)) {
                    println!("extract {}", entry.name());
                    let dest = to.join(Path::new(entry.name()).file_name().unwrap());
                    sevenz_rust::default_entry_extract_fn(entry, reader, &dest)?;
                } else {
                    std::io::copy(reader, &mut std::io::sink())?;
                };
                Ok(true)
            })
            .unwrap();
    }
    println!("========={}", BREAK_LINE);
}
///
///
///
///
pub fn build_genshin_mod(
    path: &Path,
    name: String,
    no_ramps: bool,
    variants: String,
) -> Result<(), String> {
    println!("Start build `{}`.", name);
    println!("Basic Settings");

    let dev_mode = variants.is_empty();
    println!("Dev Mode: {}", dev_mode);

    let assets_folder = path.join("assets");
    println!("Assets Folder: {}", assets_folder.as_path().display());

    let temp_vertex_folder = path.join("temp");
    println!(
        "Temp Vertex Folder: {}",
        temp_vertex_folder.as_path().display()
    );

    let output_folder = if dev_mode {
        path.join("output")
    } else {
        path.to_path_buf()
    };
    println!("Output Folder: {}", output_folder.as_path().display());
    create_output_folder(output_folder.as_path());

    let vertex_folder = output_folder.join("vertex");
    println!("Vertex Folder: {}", vertex_folder.as_path().display());

    println!("{}", BREAK_LINE);
    println!("Reading hash.json in assets folder");
    let component_list = load_hashes(&assets_folder, &name)?;
    let mut ini_config = IniConfig::new();

    for component in component_list {
        let component_name = component.component_name.unwrap_or_default();
        let classifications = component.object_classifications.unwrap_or(vec![
            "Head".to_string(),
            "Body".to_string(),
            "Extra".to_string(),
        ]);
        let current_name = name.to_string() + &component_name;
        let has_blend_vb = !component.blend_vb.is_empty();

        println!("====[{}]{}", current_name, BREAK_LINE);
        if !component.draw_vb.is_empty() {
            println!("Get stride");
            let stride = {
                let first_fmt =
                    temp_vertex_folder.join(format!("{}{}.fmt", current_name, classifications[0]));
                let file = File::open(first_fmt).unwrap();
                let reader = BufReader::new(file);

                let mut stride = String::from("0");
                for line in reader.lines() {
                    let line = line.unwrap();
                    let stride_position = line.find("stride:");
                    if let Some(pos) = stride_position {
                        stride = line[pos + 7..].trim().to_string();
                    }
                }

                stride.parse::<usize>().unwrap()
            };

            let mut offset: usize = 0;
            let mut blend: Vec<u8> = vec![];
            let mut position: Vec<u8> = vec![];
            let mut texcoord: Vec<u8> = vec![];

            ini_config.insert(
                "ib_override",
                IniChunk::new(&format!("TextureOverride{}IB", current_name))
                    .attr("hash", &component.ib.clone())
                    .attr("handling", "skip")
                    .attr("drawindexed", "auto"),
            );

            let indexes_len = component.object_indexes.len();
            let classifications_len = classifications.len();
            let classifications_last = classifications.last().unwrap().clone();

            for i in 0..indexes_len {
                let current_object = classifications.get(i).cloned().unwrap_or(format!(
                    "{}{}",
                    classifications_last,
                    i + 2 - classifications_len
                ));

                println!("Load [{}]", current_object);
                println!("Collecting VB");

                let filename = &(current_name.clone() + &current_object);
                let position_stride = if has_blend_vb {
                    println!("Splitting VB by buffer type, merging body parts");
                    collect_vb(
                        &temp_vertex_folder,
                        filename,
                        (&mut position, &mut blend, &mut texcoord),
                        stride,
                    )?;
                    40
                } else {
                    collect_vb_single(&temp_vertex_folder, filename, &mut position, stride)?;
                    stride
                };

                println!("Collecting IB");
                let ib = collect_ib(&temp_vertex_folder, filename, offset)?;

                println!("Write IB file");
                let mut file =
                    File::create(vertex_folder.join(format!("{}.ib", filename))).unwrap();
                file.write_all(&ib).unwrap();

                let mut ib_override = IniChunk::new(&format!("TextureOverride{}", filename))
                    .attr("hash", &component.ib)
                    .attr(
                        "match_first_index",
                        &component.object_indexes[i].to_string(),
                    )
                    .attr(
                        "ib",
                        &if ib.is_empty() {
                            "null".to_string()
                        } else {
                            format!("Resource{}IB", filename)
                        },
                    );

                ini_config.insert(
                    "ib_res",
                    IniChunk::new(&format!("Resource{}IB", filename))
                        .attr("type", "Buffer")
                        .attr("format", "DXGI_FORMAT_R32_UINT")
                        .attr("filename", &format!("./vertex/{}.ib", filename)),
                );

                if position.len() % position_stride != 0 {
                    eprint!("ERROR: VB buffer length does not match stride")
                }

                offset = position.len() / position_stride;

                let textures = component
                    .texture_hashes
                    .clone()
                    .and_then(|vec| Some(vec[i].clone()))
                    .unwrap_or(vec![
                        vec!["Diffuse".to_string(), ".dds".to_string(), "_".to_string()],
                        vec!["LightMap".to_string(), ".dds".to_string(), "_".to_string()],
                    ]);

                println!("Copying texture files");
                let is_face = component_name == "Face";

                let textures = if is_face {
                    let texture = textures[0].clone();
                    ini_config.insert("ib_override", ib_override);
                    ib_override =
                        IniChunk::new(&format!("TextureOverride{}{}", filename, texture[0]))
                            .attr("hash", &texture[2].to_string());
                    vec![texture]
                } else {
                    textures
                };
                for (j, texture) in textures.iter().enumerate() {
                    let layout_name = texture[0].clone();
                    if no_ramps
                        && vec!["ShadowRamp", "MetalMap", "DiffuseGuide"].contains(&&&*layout_name)
                    {
                        continue;
                    }

                    let full_filename = format!("{}{}{}", filename, layout_name, texture[1]);

                    ib_override = ib_override.attr(
                        &format!("ps-t{}", j),
                        &format!("Resource{}{}", filename, layout_name),
                    );

                    ini_config.insert(
                        "tex_res",
                        IniChunk::new(&format!("Resource{}{}", filename, layout_name))
                            .attr("filename", &format!("./assets/{}", full_filename)),
                    );
                    if dev_mode {
                        fs::copy(
                            assets_folder.join(&full_filename),
                            output_folder.join("assets").join(&full_filename),
                        )
                        .unwrap();
                    }
                }
                ini_config.insert("ib_override", ib_override)
            }
            if !component.blend_vb.is_empty() {
                println!("Writing merged buffer files");
                let mut file =
                    File::create(vertex_folder.join(format!("{}Position.buf", current_name)))
                        .unwrap();
                file.write_all(&position).unwrap();
                let mut file =
                    File::create(vertex_folder.join(format!("{}Blend.buf", current_name))).unwrap();
                file.write_all(&blend).unwrap();
                let mut file =
                    File::create(vertex_folder.join(format!("{}Texcoord.buf", current_name)))
                        .unwrap();
                file.write_all(&texcoord).unwrap();

                let chunk = IniChunk::new(&format!("TextureOverride{}Position", current_name))
                    .attr("hash", &component.position_vb)
                    .attr("vb0", &format!("Resource{}Position", current_name));

                ini_config.insert(
                    "vb_override",
                    if !variants.is_empty() {
                        chunk.attr("$active", "1")
                    } else {
                        chunk
                    },
                );

                ini_config.insert(
                    "vb_override",
                    IniChunk::new(&format!("TextureOverride{}Blend", current_name))
                        .attr("hash", &component.blend_vb)
                        .attr("vb1", &format!("Resource{}Blend", current_name))
                        .attr("handling", "skip")
                        .attr("draw", &format!("{}, 0", position.len() / 40)),
                );

                ini_config.insert(
                    "vb_override",
                    IniChunk::new(&format!("TextureOverride{}Texcoord", current_name))
                        .attr("hash", &component.texcoord_vb)
                        .attr("vb1", &format!("Resource{}Texcoord", current_name)),
                );

                ini_config.insert(
                    "vb_override",
                    IniChunk::new(&format!("TextureOverride{}VertexLimitRaise", current_name))
                        .attr("hash", &component.draw_vb),
                );

                ini_config.insert(
                    "vb_res",
                    IniChunk::new(&format!("Resource{}Position", current_name))
                        .attr("type", "Buffer")
                        .attr("stride", "40")
                        .attr(
                            "filename",
                            &format!("./vertex/{}Position.buf", current_name),
                        ),
                );

                ini_config.insert(
                    "vb_res",
                    IniChunk::new(&format!("Resource{}Blend", current_name))
                        .attr("type", "Buffer")
                        .attr("stride", "32")
                        .attr("filename", &format!("./vertex/{}Blend.buf", current_name)),
                );

                ini_config.insert(
                    "vb_res",
                    IniChunk::new(&format!("Resource{}Texcoord", current_name))
                        .attr("type", "Buffer")
                        .attr("stride", &(stride - 72).to_string())
                        .attr(
                            "filename",
                            &format!("./vertex/{}Texcoord.buf", current_name),
                        ),
                );
            } else {
                let mut file =
                    File::create(output_folder.join(format!("./vertex/{}.buf", current_name)))
                        .unwrap();
                file.write_all(&position).unwrap();

                let mut chunk = IniChunk::new(&format!("TextureOverride{}", current_name))
                    .attr("hash", &component.draw_vb)
                    .attr("vb0", &format!("Resource{}", current_name));
                if !variants.is_empty() {
                    chunk = chunk.attr("$active", "1")
                };
                ini_config.insert("vb_override", chunk);

                ini_config.insert(
                    "vb_res",
                    IniChunk::new(&format!("Resource{}", current_name))
                        .attr("type", "Buffer")
                        .attr("stride", &stride.to_string())
                        .attr("filename", &format!("./vertex/{}.buf", current_name)),
                );
            }
        } else {
            let indexes_len = component.object_indexes.len();
            for i in 0..indexes_len {
                let current_object = if i <= 2 {
                    classifications[i].clone()
                } else {
                    format!("{}{}", classifications[2], i - 1)
                };

                let filename = &(current_name.clone() + &current_object);
                println!("Texture override only on {}", current_object);
                let textures = component
                    .texture_hashes
                    .clone()
                    .and_then(|vec| Some(vec[i].clone()))
                    .unwrap_or(vec![
                        vec!["Diffuse".to_string(), ".dds".to_string(), "_".to_string()],
                        vec!["LightMap".to_string(), ".dds".to_string(), "_".to_string()],
                    ]);

                println!("Copying texture files");
                let is_face = component_name == "Face";
                let textures = if is_face {
                    vec![textures[0].clone()]
                } else {
                    textures
                };

                for (j, texture) in textures.iter().enumerate() {
                    let layout_name = texture[0].clone();
                    if no_ramps
                        && vec!["ShadowRamp", "MetalMap", "DiffuseGuide"].contains(&&&*layout_name)
                    {
                        continue;
                    }

                    let full_filename = format!("{}{}{}", filename, layout_name, texture[1]);

                    ini_config.insert(
                        "ib_override",
                        IniChunk::new(&format!("TextureOverride{}{}", filename, layout_name))
                            .attr("hash", &texture[2])
                            .attr(
                                &format!("ps-t{}", j),
                                &format!("Resource{}{}", filename, layout_name),
                            ),
                    );
                    ini_config.insert(
                        "tex_res",
                        IniChunk::new(&format!("Resource{}{}", filename, layout_name))
                            .attr("filename", &format!("./assets/{}", &full_filename)),
                    );
                    fs::copy(
                        assets_folder.join(&full_filename),
                        output_folder.join("assets").join(&full_filename),
                    )
                    .unwrap();
                }
            }
        }
    }

    println!("collect finished");

    if !variants.is_empty() {
        ini_config.insert(
            "constant",
            IniChunk::new("Constants")
                .attr("global $active", "0")
                .attr("global $variantsinfo", "0"),
        );
        ini_config.insert(
            "constant",
            IniChunk::new("Present")
                .attr("post $active", "0")
                .attr("run", "CommandListVariantsInfo"),
        );

        ini_config.insert(
            "command",
            IniChunk::new("CommandListVariantsInfo")
                .push("if $variantsinfo == 0 && $active == 1")
                .push("pre Resource\\ShaderFixes\\help.ini\\Notification = ResourceVariantsInfo")
                .push("pre run = CustomShader\\ShaderFixes\\help.ini\\FormatText")
                .push("pre $\\ShaderFixes\\help.ini\\notification_timeout = time + 8.0")
                .push("$variantsinfo = 1")
                .push("endif"),
        );

        ini_config.insert(
            "other",
            IniChunk::new("ResourceVariantsInfo").push(&format!(
                "type = Buffer\ndata = \"{} (by xiaoeyun)\"\n\n",
                variants
            )),
        );
    }

    println!("Generating .ini file");
    let ini_text = ini_config.format(
        ";Constants -------------------------
<constant>
;Overrides -----------------------
<vb_override>
<ib_override>
;CommandList ---------------------
<command>
;Resources -----------------------
<vb_res>
<ib_res>
<tex_res>
<other>
;.ini generated by HornyLoader (Discord `xiaoeyun`)
; based GIMI (Genshin-Impact-Model-Importer)",
    );

    fs::write(output_folder.join(format!("{}.ini", name)), ini_text).unwrap();

    Ok(())
}

fn collect_vb(
    vertex_path: &Path,
    name: &str,
    bytes: (&mut Vec<u8>, &mut Vec<u8>, &mut Vec<u8>),
    stride: usize,
) -> Result<(), String> {
    let mut file: File = File::open(vertex_path.join(name.to_string() + ".vb")).unwrap();
    let mut buff = vec![];
    file.read_to_end(&mut buff).unwrap();
    let buff_len = buff.len();
    let mut i = 0;
    while i < buff_len {
        bytes.0.extend(&buff[i..i + 40]);
        bytes.1.extend(&buff[i + 40..i + 72]);
        bytes.2.extend(&buff[i + 72..i + stride]);
        i += stride;
    }
    Ok(())
}

fn collect_ib(vertex_path: &Path, name: &str, offset: usize) -> Result<Vec<u8>, String> {
    let mut file = File::open(vertex_path.join(name.to_string() + ".ib")).unwrap();
    let mut buff = vec![];
    file.read_to_end(&mut buff).unwrap();
    let buff_len = buff.len();
    let mut ib = vec![];
    let mut i = 0;
    while i < buff_len {
        let mut value = 0;
        buff[i..i + 4]
            .iter()
            .rev()
            .for_each(|v| value = (value << 8) + *v as usize);
        value += offset;
        let mut offset_bytes = [0_u8; 4];
        for i in 0..4 {
            *&mut offset_bytes[i] = (value % 256) as u8;
            value >>= 8
        }

        ib.extend(&offset_bytes);
        i += 4;
    }
    return Ok(ib);
}

fn collect_vb_single(
    vertex_path: &Path,
    name: &str,
    bytes: &mut Vec<u8>,
    stride: usize,
) -> Result<(), String> {
    let mut file = File::open(vertex_path.join(name.to_string() + ".vb")).unwrap();
    let mut buff = vec![];
    file.read_to_end(&mut buff).unwrap();
    let buff_len = buff.len();
    let mut i = 0;
    while i < buff_len {
        bytes.extend(&buff[i..i + stride]);
        i += stride;
    }
    Ok(())
}

fn load_hashes(assets_path: &Path, name: &str) -> Result<Vec<Component>, String> {
    let json_path = assets_path.join("hash.json");
    let older_json_path = assets_path.join("hash_info.json");
    if json_path.exists() {
        let file = File::open(json_path).unwrap();
        serde_json::from_reader(file).map_err(|e| e.to_string())
    } else if older_json_path.exists() {
        eprint!("[warning] Could not find hash.json in assets folder. fallback to hash_info.json");
        let file = File::open(older_json_path).unwrap();
        let mut object: HashMap<String, Component> = serde_json::from_reader(file).unwrap();
        let component = object
            .remove(name)
            .ok_or(format!("Cannot find \"{}\" in hash_info.json", name))?;
        Ok(vec![component])
    } else {
        Err("Cannot find hash information, check hash.json in assets".to_string())
    }
}

fn create_output_folder(output: &Path) {
    if !output.exists() {
        println!("Generate mod folder");
        fs::create_dir(output).unwrap();
    }

    let path = output.join("vertex");
    if !path.exists() {
        println!("Generate mod/vertex folder");
        fs::create_dir(path).unwrap();
    }

    let path = output.join("assets");
    if !path.exists() {
        println!("Generate mod/assets folder");
        fs::create_dir(path).unwrap();
    }
}

#[derive(Debug, Clone)]
struct IniConfig(HashMap<String, Vec<IniChunk>>);

impl IniConfig {
    pub fn new() -> IniConfig {
        IniConfig(HashMap::new())
    }
    pub fn insert(&mut self, name: &str, chunk: IniChunk) {
        self.0.entry(name.to_string()).or_default().push(chunk);
    }
    pub fn format(&self, format_string: &str) -> String {
        let mut text = format_string.to_string();
        for (name, chunks) in self.0.clone() {
            text = text.replace(
                &format!("<{}>", name),
                &chunks
                    .iter()
                    .map(|chunks| chunks.format())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
        }
        text
    }
}

#[derive(Debug, Clone)]
struct IniChunk {
    name: String,
    attrs: Vec<String>,
}

impl IniChunk {
    pub fn new(name: &str) -> IniChunk {
        IniChunk {
            name: name.to_string(),
            attrs: Vec::new(),
        }
    }
    pub fn attr(mut self, name: &str, value: &str) -> IniChunk {
        self.attrs.push(format!("{} = {}", name, value));
        self
    }
    pub fn push(mut self, text: &str) -> IniChunk {
        self.attrs.push(text.to_string());
        self
    }
    pub fn format(&self) -> String {
        format!("[{}]\n{}\n", self.name, self.attrs.join("\n"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModConfig {
    name: String,
    options: Vec<(String, Vec<String>)>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Component {
    component_name: Option<String>,
    root_vs: Option<String>,
    draw_vb: String,
    position_vb: String,
    blend_vb: String,
    texcoord_vb: String,
    ib: String,
    object_indexes: Vec<usize>,
    object_classifications: Option<Vec<String>>,
    texture_hashes: Option<Vec<Vec<Vec<String>>>>,
    first_vs: String,
}
