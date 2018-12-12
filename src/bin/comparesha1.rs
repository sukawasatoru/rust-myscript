use dotenv;
use env_logger;
use log::{debug, info};

#[derive(Debug)]
struct Entity {
    hash: String,
    name: String,
}

impl Entity {
    fn new() -> Entity {
        Entity {
            hash: String::new(),
            name: String::new(),
        }
    }
}

#[derive(Debug)]
struct IllegalFormatError {
    line: String,
}

impl IllegalFormatError {
    fn new(line: &str) -> IllegalFormatError {
        IllegalFormatError {
            line: line.to_string(),
        }
    }

    fn as_str(&self) -> &'static str {
        "input format of sha1sum unexpected"
    }
}

impl std::error::Error for IllegalFormatError {
    fn description(&self) -> &str {
        self.as_str()
    }

    fn cause(&self) -> Option<&std::error::Error> {
        None
    }
}

impl std::fmt::Display for IllegalFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}: {}", self.as_str(), self.line)
    }
}

fn main() {
    dotenv::dotenv().ok();
    env_logger::init();
    info!("Hello");

    loop {
        match read_pair() {
            Ok(Some((lh, rh))) => {
                if lh.hash.eq(&rh.hash) {
                    println!("OK hash={} lh={} rh={}", lh.hash, lh.name, rh.name);
                } else {
                    println!("NG");
                }
                debug!("lh={:?} rh={:?}", lh, rh);
            }
            Ok(None) => {
                info!("none");
                break;
            }
            Err(e) => {
                info!("err {:?}", e);
                break;
            }
        }
    }

    info!("Bye");
}

fn read_pair() -> Result<Option<(Entity, Entity)>, std::io::Error> {
    let mut lh = String::new();
    std::io::stdin().read_line(&mut lh)?;

    if lh.is_empty() {
        return Ok(None);
    }

    let l_len = lh.len() - 1;
    lh.truncate(l_len);

    if lh.is_empty() {
        return Ok(None);
    }

    let l_entity = create_entity(&lh).unwrap();

    let mut rh = String::new();
    std::io::stdin().read_line(&mut rh)?;

    if rh.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, ""));
    }

    let r_len = rh.len() - 1;
    rh.truncate(r_len);

    if rh.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, ""));
    }

    let r_entity = create_entity(&rh).unwrap();

    Ok(Some((l_entity, r_entity)))
}

fn create_entity(input: &str) -> Result<Entity, IllegalFormatError> {
    let list: Vec<&str> = input.split(' ').collect();
    let mut ret = Entity::new();
    for part in &list {
        if !part.is_empty() {
            if ret.hash.is_empty() {
                ret.hash = part.to_string();
            } else {
                ret.name = part.to_string();
            }
        }
    }

    if ret.hash.is_empty() || ret.name.is_empty() {
        return Err(IllegalFormatError::new(&input));
    }

    Ok(ret)
}
