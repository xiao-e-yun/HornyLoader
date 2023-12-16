use std::thread;
use std::time::Duration;
use std::{
    fmt::Display,
    io::{self, Write},
};

const BREAK_LINE: &'static str = "===================================================";

mod dev;
mod load;

fn main() {
    loop {
        println!("|HornyLoader|");
        println!("A Genshin Mod Tools for build & load");
        println!("Power By xiao-e-yun (https://github.com/xiao-e-yun)");
        println!("{}", BREAK_LINE);

        let index = list_and_choose(&"Functions", vec!["Load Mod", "Dev Mode", "Exit"], true);

        match index {
            0 => load::main(),
            1 => dev::main(),
            2 => {
                break;
            }
            _ => unreachable!(),
        }
        .unwrap_or_else(|info| {
            eprintln!(
                "==Error============================================\n{}\n{}",
                info, BREAK_LINE
            )
        });

        thread::sleep(Duration::from_millis(500));
    }
}

//=================================================================
// Utils
//=================================================================
pub fn list_and_choose(desc: impl Display, list: Vec<impl Display>, default: bool) -> usize {
    loop {
        if !desc.to_string().is_empty() {
            println!("{}", desc)
        };
        println!("Please choose one");
        for (i, command) in list.iter().enumerate() {
            let is_default = if i == 0 && default { "(Default)" } else { "" };
            println!("{}. {} {}", i, command, is_default);
        }

        let input = read_input();
        let choose = if default && input.is_empty() {
            Some(0)
        } else {
            list.iter()
                .position(|v| input.to_lowercase() == v.to_string().to_lowercase())
                .or(input.parse::<usize>().ok().and_then(|i| {
                    if i < list.len() {
                        Some(i)
                    } else {
                        None
                    }
                }))
        };

        match choose {
            Some(index) => {
                println!("Choose `{}`\n{}", list.get(index).unwrap(), BREAK_LINE);
                break index;
            }
            None => {
                println!("Wrong input");
                println!("Please retry");
                println!("{}", BREAK_LINE);
            }
        }
    }
}

fn read_input() -> String {
    let mut input_text = String::new();
    print!("> ");
    io::stdout().flush().unwrap();
    io::stdin()
        .read_line(&mut input_text)
        .expect("Failed to read line");
    input_text.trim().to_string()
}
