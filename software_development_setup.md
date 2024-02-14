We are going to use an ESP32-H2 SoC with Rust embedded Toolchain with standard library.
For that and this project we need:

- Rust compiler (see www.rust-lang.org) 
- Rust embedded toolchain for ESP32-H2 for projects using std lib
    Follow the instructions for std projects in this book chapter:
    [The Rust on ESP Book - Setting Up a Development Environment](https://esp-rs.github.io/book/installation/index.html)
    Notes on this:
      - ignore sections for `no_std` 
      - When it comes to the step `espup install`, you should consider using `espup install --targets esp32h2` instead, to avoid installing lots of unnecessary dependencies for unused Espressif targets. 
- `source ~/export-esp.sh; cargo build` 