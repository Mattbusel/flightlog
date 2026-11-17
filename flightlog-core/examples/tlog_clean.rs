//! Writes only the valid records of a tlog (drops console text and damage),
//! so pymavlink and flightlog read exactly the same frames.
//! Used to make the `*-clean.tlog` reference logs:
//! `cargo run -p flightlog-core --release --example tlog_clean -- in.tlog out.tlog`

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1])?;
    let mut out =
        mavlink_tlog::TlogWriter::new(std::io::BufWriter::new(std::fs::File::create(&args[2])?));
    let mut log = mavlink_tlog::TlogSlice::new(&bytes);
    for rec in log.by_ref() {
        if mavlink_tlog::message_info(rec.frame.msg_id()).is_some() {
            out.write_frame(rec.timestamp_us, rec.frame.as_bytes())?;
        }
    }
    out.flush()?;
    eprintln!("{:?}", log.stats());
    Ok(())
}
