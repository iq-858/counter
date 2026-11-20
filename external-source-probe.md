# External source availability probe (2026-11-19)

Method: read-only HTTP HEAD requests from the OpenCode Code Mode runtime on 2026-11-19. Redirects followed. These are availability and content-type observations, **not** decode/playback results. Codec/resolution labels come from the source site's own asset table (`https://test-videos.co.uk/`), not from a local `ffprobe` of the remote bytes.

| Source | Content type | Bytes | Status | Site-declared format |
| --- | --- | ---: | ---: | --- |
| `https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/360/Big_Buck_Bunny_360_10s_1MB.mp4` | `video/mp4` | 991017 | 200 | MP4 / H.264 / 640x360 |
| `https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/720/Big_Buck_Bunny_720_10s_2MB.mp4` | `video/mp4` | 1978137 | 200 | MP4 / H.264 / 1280x720 |
| `https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/1080/Big_Buck_Bunny_1080_10s_5MB.mp4` | `video/mp4` | 5238714 | 200 | MP4 / H.264 / 1920x1080 |
| `https://test-videos.co.uk/vids/bigbuckbunny/mp4/av1/1080/Big_Buck_Bunny_1080_10s_5MB.mp4` | `video/mp4` | 5251990 | 200 | MP4 / AV1 / 1920x1080 |
| `https://test-videos.co.uk/vids/bigbuckbunny/webm/vp9/720/Big_Buck_Bunny_720_10s_2MB.webm` | `video/webm` | 2100629 | 200 | WebM / VP9 / 1280x720 |
| `https://test-videos.co.uk/vids/bigbuckbunny/mkv/1080/Big_Buck_Bunny_1080_10s_5MB.mkv` | `video/x-matroska` | 4816236 | 200 | MKV / H.264 / 1920x1080 |
| `https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8` | `audio/mpegurl` | 752 | 200 | HLS playlist |

Also checked earlier in the same session: `https://test-videos.co.uk/vids/bigbuckbunny/webm/vp8/720/Big_Buck_Bunny_720_10s_1MB.webm` returned `video/webm`, 1058683 bytes, 200. The site's AVI sample URL pattern tried (`.../avi/mpeg4/360/...`) returned 404 and is not usable. `https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4` was unreachable from this runtime (`getaddrinfo ENOTFOUND`) and is not usable as evidence here.
