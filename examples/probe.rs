use mp4::Mp4Reader;
use std::fs::File;

fn main() {
    for name in ["bbb10", "flower", "sintel", "bunny", "w3c"] {
        let path = format!("assets/videos/{}.mp4", name);
        let f = File::open(&path).unwrap();
        let size = f.metadata().unwrap().len();
        let mp4 = Mp4Reader::read_header(f, size).unwrap();
        let mut info = String::new();
        for (id, t) in mp4.tracks().iter() {
            let media = if matches!(t.track_type(), Ok(mp4::TrackType::Video)) {
                "video"
            } else if matches!(t.track_type(), Ok(mp4::TrackType::Audio)) {
                "audio"
            } else {
                "?"
            };
            info.push_str(&format!(
                "[track {id} {media} {}x{} ts={}] ",
                t.width(),
                t.height(),
                t.timescale(),
            ));
        }
        println!("{}: dur={:?} {}", name, mp4.duration(), info);
    }
}
