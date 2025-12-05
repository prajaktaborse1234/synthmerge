use std::env;

include!("src/main_args.rs");

fn main() -> std::io::Result<()> {
    if env::var("PROFILE").unwrap_or_default() == "release" {
        let out_dir = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").ok_or(std::io::ErrorKind::NotFound)?,
        )
        .parent()
        .ok_or(std::io::ErrorKind::NotFound)?
        .join("man");
        std::fs::create_dir_all(&out_dir)?;

        use clap::CommandFactory;
        let cmd = Args::command();

        use clap_mangen::Man;
        let man = Man::new(cmd);
        let mut buffer: Vec<u8> = Default::default();
        man.render(&mut buffer)?;

        std::fs::write(
            out_dir.join(vec![env!("CARGO_PKG_NAME"), ".1"].join("")),
            buffer,
        )?;
    }

    Ok(())
}

// Local Variables:
// rust-format-on-save: t
// End:
