fn main() {
    let paths = std::env::var("PATH").unwrap();

    let mut index = 0;
    let mut dest = [""; 1024];
    for path in paths.split(':') {
        if !dest.contains(&path) {
            dest[index] = path;
            index += 1;
        }
    }

    print!("{}", dest[..index].join(":"));
}
