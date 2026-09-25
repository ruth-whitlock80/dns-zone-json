use std::env;
use std::fs;
use std::process;

mod json;
mod record;
mod zone;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut input_path: Option<String> = None;
    let mut origin = ".".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--origin" => {
                i += 1;
                match args.get(i) {
                    Some(value) => origin = value.clone(),
                    None => {
                        eprintln!("--origin needs a value");
                        process::exit(2);
                    }
                }
            }
            "-h" | "--help" => {
                print_usage();
                return;
            }
            other => {
                if input_path.is_some() {
                    eprintln!("unexpected argument \"{}\"", other);
                    process::exit(2);
                }
                input_path = Some(other.to_string());
            }
        }
        i += 1;
    }

    let path = match input_path {
        Some(p) => p,
        None => {
            print_usage();
            process::exit(2);
        }
    };

    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not read {}: {}", path, e);
            process::exit(1);
        }
    };

    let result = if path.ends_with(".json") {
        convert_json_to_zone(&contents)
    } else {
        convert_zone_to_json(&contents, &origin)
    };

    match result {
        Ok(output) => print!("{}", output),
        Err(message) => {
            eprintln!("{}", message);
            process::exit(1);
        }
    }
}

fn convert_zone_to_json(contents: &str, origin: &str) -> Result<String, String> {
    let records = zone::parse_zone(contents, origin).map_err(|e| e.to_string())?;
    let value = record::records_to_json(&records);
    let mut text = json::to_string_pretty(&value);
    text.push('\n');
    Ok(text)
}

fn convert_json_to_zone(contents: &str) -> Result<String, String> {
    let value = json::parse(contents)?;
    let records = record::records_from_json(&value)?;
    Ok(zone::write_zone(&records))
}

fn print_usage() {
    eprintln!("usage: dnsconv <file.zone|file.json> [--origin <name>]");
    eprintln!();
    eprintln!("Converts a BIND-style zone file to JSON, or JSON back to a zone file.");
    eprintln!("Direction is chosen by the input file's extension (.json means JSON in,");
    eprintln!("anything else is read as a zone file). --origin sets the starting");
    eprintln!("$ORIGIN for zone files that use relative names before their first");
    eprintln!("$ORIGIN directive.");
}
