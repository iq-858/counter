# توثيق التكافؤ بين المنصات — `cranpose-video`

> مستند تدقيق ثابت (source-grounded) يصف ما تفعله المنصات الست فعليًا في هذه النسخة من
> المستودع، لا ما يُتوقع منها نظريًا. اللغة عربية، والمصطلحات التقنية (أسماء الأنواع،
> الدوال، الميزات، البنى) تُترك بالإنجليزية كما هي في الكود.

## 0. نطاق الفحص واصطلاح الاستشهاد

الملفات المقروءة بالكامل أو بعين التدقيق:

- `src/platform/mod.rs`, `src/platform/windows.rs`, `src/platform/windows/mf_audio.rs`,
  `src/platform/apple.rs`, `src/platform/apple_avplayer.rs`, `src/platform/ios_route.rs`,
  `src/platform/android.rs`, `src/platform/linux.rs`, `src/platform/web.rs`,
  `src/platform/unsupported.rs`
- `src/audio.rs`, `src/arbitration.rs`, `src/session.rs`, `src/decoder.rs`, `src/source.rs`,
  `src/lib.rs`, `src/player.rs`, `src/player/threaded.rs`, `src/player/inline.rs`,
  `src/hls/mod.rs`, `src/hls/playlist.rs`, `src/hls/tfdt.rs`, `Cargo.toml`, `README.md`

قواعد الاستشهاد في الجداول أدناه:

- المسارات نسبية إلى جذر ال crate (`D:\counter\cranpose-video`)، وأرقام الأسطر 1-based.
- `NONE` تعني: لا يوجد تجاوز (override) ولا استدعاء لهذه الميزة في ملفات المنصة،
  وسلوكها هو القيمة الافتراضية في `VideoDecoder` (المشار إليها عند الحاجة).
- `HYPOTHESIS` تعني: الاستدلال مستند إلى كون ال backend هو حزمة النظام، لكن الكود لا
  يذكر قائمة صريحة (مثل قائمة الكodecs على Apple).
- `DEFAULT` تعني أن المنصة تستخدم تنفيذ التrait الافتراضي دون تعديل.

قيود البناء (تغيّر المصفوفة جذريًا حسب ال features):

- `audio` module يُبنى فقط مع `native-audio` **وليس** على wasm — `src/lib.rs:4-5`.
- `arbitration` يُبنى على كل المنصات عدا Web — `src/lib.rs:6-10`.
- `session` يُبنى على Windows/macOS/iOS/Android فقط (لا Linux، لا Web) —
  `src/lib.rs:28-29`; البناء على Linux مستبعد صراحة في `src/session.rs:17-18`.
- `hls` module يُبنى على Windows/Android/macOS/iOS/Linux-with-feature فقط —
  `src/lib.rs:15-22`.
- Linux يحتاج `linux-gstreamer` وإلا يستخدم `unsupported` —
  `Cargo.toml:37`, `src/platform/mod.rs:91-92`, `src/platform/mod.rs:138-156`,
  `src/platform/unsupported.rs:9-16`.
- `Audio::attach_session` / `recover` في التrait نفسها مستبعدة على Linux و wasm —
  `src/decoder.rs:221-234`; حلقة الاسترداد في المشغّل مستبودة عليهما أيضًا —
  `src/player/threaded.rs:218-219`, `src/player/threaded.rs:240-255`.

---

## 1. مصفوفة الميزات (Feature-by-feature matrix)

الأعمدة: **Windows | macOS | iOS | Android | Linux | Web**.

| # | الميزة | Windows | macOS | iOS | Android | Linux | Web |
|---|--------|---------|-------|-----|---------|-------|-----|
| 1 | **Video formats / decoder stack** | Media Foundation؛ القائمة المذكورة صراحة: H.264, HEVC, VP9, AV1, MPEG-2 — `src/platform/windows.rs:1-8`, `Cargo.toml:62-63`. "كل container/codec يدعمه مشغّل النظام" بما فيه MPEG-TS — `README.md:240`. NV12 مفضّل وRGB32 بديل — `windows.rs:210-221`. | AVFoundation/`AVAssetReader`: NV12 مطلوب أولًا ثم BGRA بديل — `src/platform/apple.rs:1-5`, `apple.rs:579-588`. القائمة الدقيقة للكودكس `HYPOTHESIS` (تتبع إطار العمل). `AVAssetReader` لا يقرأ MPEG-TS — `src/platform/mod.rs:107-110`. | مثل macOS للملفات المحلية — `src/platform/apple.rs:43-44`, `mod.rs:72-73`. مسار HLS يستخدم AVPlayer لأن `AVAssetReader` لا يفتح TS — `mod.rs:75-78`, `mod.rs:107-110`. القائمة `HYPOTHESIS`. | MediaExtractor + MediaCodec عبر `createDecoderByType(mime)` — `src/platform/android.rs:1-8`, `android.rs:1196-1214`; أي track يبدأ بـ `video/` تقبله المنصة — `android.rs:1183-1184`. الإخراج YUV420 → NV12/RGBA — `android.rs:725-869`. القائمة تعتمد على الجهاز `HYPOTHESIS`; ترتيب NV21/Vu `HYPOTHESIS` — `README.md:593`. | GStreamer `uridecodebin` يختار ما تثبّته الحزمة من plugins — `src/platform/linux.rs:1-8`, `linux.rs:45-53`.NV12 عبر `videoconvert` — `linux.rs:69-76`. لا يوجد decoder إطلاقًا بدون feature — `src/platform/unsupported.rs:9-16`. | عنصر `<video>` الخاص بالمتصفح — `src/platform/web.rs:1-3`, `web.rs:41-47`. الصيغ = ما يدعمه المتصفح `HYPOTHESIS`. بايتات تُوسم `video/mp4` — `web.rs:101-110`. |
| 2 | **HLS support path** | داخلي: `platform::open` يكتشف `.m3u8` ويستدعي `hls::open` — `src/platform/mod.rs:167-188`؛ يضمّ مقاطع في `TempFile` ثم `open_hls_window` → `open_native` — `src/hls/mod.rs:658-668`, `mod.rs:130-135`. MF يقرأ TS — `mod.rs:109`. | نفس المسار الداخلي — `mod.rs:178-187`; fMP4 يعمل مع `AVAssetReader` على macOS — `mod.rs:109-110`. TS كملف مستقل غير قابل للفتح عبر `AVAssetReader` — `mod.rs:107-110`. | `.m3u8` يُمرّر كـ URL أصلي إلى AVPlayer: `open_hls_direct` — `src/platform/mod.rs:111-128`, `mod.rs:178-185`; و`VideoPlayer::open_hls` يذهب مباشرة إليه — `src/player.rs:873-874`. مسار الـ window ما زال موجودًا لكنه ليس المسار الرئيسي — `mod.rs:105-116`. | داخلي: `mod.rs:178-187` + `src/hls/mod.rs:667-668` → MediaCodec على الملف المدمج — `android.rs:978-987`. الـ demuxer يفتح URL كملف واحد — `mod.rs:163-164`. | داخلي فقط مع `linux-gstreamer`: `src/lib.rs:15-22`, `mod.rs:176`, ثم `file://` URI — `linux.rs:36-43`. | المتصفح وحده: الموجّه لا يتدخل — `mod.rs:166`. `canPlayType(HLS_TYPE)` — `web.rs:35-36`, `web.rs:77-83`; وإلا يُحمّل `hls.js` من CDN — `web.rs:38-39`, `web.rs:113-230`. |
| 3 | **HLS audio path** | الصوت داخل نافذة MF نفسها على clock واحد — `README.md:240`, `src/hls/mod.rs:963-965`. TS مدعوم صوتيًا هنا فقط — `README.md:251`. rendition صوتي منفصل: `src/hls/mod.rs:658-665`, `hls/mod.rs:676-683` (مشروط بـ `carries_audio()` — `hls/mod.rs:963-965`). | صوت عام (SoftwareMediaPlayer) يفتح الملف المدمج — `src/hls/mod.rs:966-985`. fMP4 فقط؛ صوت TS يبقى صامتًا — `README.md:251`, `mod.rs:109-110`. | AVPlayer هو المصدر الوحيد للصوت — `src/platform/apple_avplayer.rs:71-78`, `apple_avplayer.rs:122-126`. | صوت عام عبر `hls::Stream::audio` — `src/hls/mod.rs:949-985`; MPEG-TS audio صامت — `README.md:251`. | صوت عام — `src/hls/mod.rs:949-985`; MPEG-TS audio صامت — `README.md:251`; وبلا `session` أصلًا — `src/lib.rs:28-29`. | عنصر `<video>` / `hls.js` يملك الصوت — `web.rs:421-435`. |
| 4 | **Audio owner (platform vs generic)** | **Platform**: `mf_audio::Audio`؛ source reader واحد يملك المسارين — `src/platform/windows.rs:93-97`, `windows.rs:806-817`, `src/platform/windows/mf_audio.rs:1-20`. | **Generic**: `audio::wrap_direct` يزاوج `SoftwareMediaPlayer` — `src/audio.rs:608-624`, `src/player.rs:837-838`; `apple.rs` لا يعرّف `carries_audio()`. | **Generic** للملفات المحلية — `src/audio.rs:608-624`; **platform** (AVPlayer) لـ HLS — `apple_avplayer.rs:71-78`. | **Generic** — `src/audio.rs:608-624`; توجيه AAudio يتم في طبقة المنصة لا هنا — `Cargo.toml:88-92`. | **Generic** مع cpal — `src/audio.rs:19-31`, `audio.rs:371-421`, `Cargo.toml:88-95`. | **Platform** (browser element)؛ وحدة `audio` غير مبنية — `src/lib.rs:4`, `web.rs:421-435`. |
| 5 | **`carries_audio()` value** | `true` إذا `native-audio` و(`audio.is_some() \|\| audio_pending \|\| source_has_audio`) — `windows.rs:1054-1062`؛ وإلا `DEFAULT=false` — `src/decoder.rs:184`. | `DEFAULT=false` (لا override) — `src/decoder.rs:184`, المقارنة في `src/audio.rs:615`. | ملف محلي `DEFAULT=false` — `decoder.rs:184`; AVPlayer `true` — `apple_avplayer.rs:218-222`; HLS Stream يمرّر قيمة نافذته — `src/hls/mod.rs:1337-1341`. | `DEFAULT=false` — `decoder.rs:184`; HLS Stream يمرّر قيمة نافذته (MediaCodec) = false. | `DEFAULT=false` — `decoder.rs:184`. | `false` تقنيًا — `decoder.rs:184` — **رغم أن العنصر يملك الصوت** — `web.rs:421-435`; القيمة لا تُستخدم لأن `audio` غير مبني — `src/lib.rs:4`. |
| 6 | **Arbitration participation** | **لا يشارك** على مسار MF: لا `set_audio_group` في `windows.rs` → `DEFAULT` no-op — `src/decoder.rs:160`; `mf_audio` يضبط mute/volume فقط — `mf_audio.rs:1334-1340`. الـ generic arbitrates — `audio.rs:545-589` — لكنه يتنحى بسبب `carries_audio=true`. | نعم، عبر generic `Audio`: claim/arbitrate — `src/audio.rs:432-438`, `audio.rs:545-589`, `audio.rs:748-751`. | ملفات: generic كما في macOS؛ HLS AVPlayer يشارك مباشرة — `apple_avplayer.rs:40`, `apple_avplayer.rs:228-237`, `apple_avplayer.rs:246-256`, `apple_avplayer.rs:288-296`, `apple_avplayer.rs:311-315`. | نعم عبر generic — `audio.rs:748-751`. | نعم عبر generic — `audio.rs:748-751` (لكن بلا session). | **NONE** — `arbitration` غير مبني — `src/lib.rs:6-10`. |
| 6b | **Arbitration على HLS Stream** | `set_audio_group` **NONE** في `src/hls/mod.rs` → `DEFAULT` no-op — `decoder.rs:160`; صوت النافذة من MF. | نفس الشيء: لا تمرير → `decoder.rs:160`; صوت generic داخل `hls::Stream` يبقى في `DEFAULT_GROUP` — `src/audio.rs:167`, `src/arbitration.rs:23`. | نفس الشيء؛ AVPlayer المباشر (عبر `open_hls`) يشارك — `apple_avplayer.rs:228-237`، بينما `hls::Stream` لا. | نفسه — `hls/mod.rs` بلا `set_audio_group`. | نفسه — `hls/mod.rs` بلا `set_audio_group`. | لا يوجد arbitration أصلًا — `src/lib.rs:6-10`. |
| 7 | **Route-change detection** | نعم، آليّة MF/CPAL مزدوجة: error callback `StreamInvalidated`/`DeviceNotAvailable` — `mf_audio.rs:95-106`, `mf_audio.rs:945-956`؛ وفحص استباقي لـ default device في `pump` — `mf_audio.rs:1132-1196`; إعادة ربط + renegotiate — `mf_audio.rs:1000-1120`. | استطلاع cpal عام فقط — `src/audio.rs:27-31`, `audio.rs:371-421`, `audio.rs:48-51`; لا observer على مستوى النظام — `NONE` (`ios_route` مقصور على iOS — `mod.rs:82-83`). | **نعم آليًا**: `AVAudioSessionRouteChangeNotification` — `src/platform/ios_route.rs:1-43`, ثُبّت في `platform::open` — `mod.rs:168-170` → `PlaybackSession::route_changed_all()` — `src/session.rs:189-201`. لكن AVPlayer HLS لا يستقبل `attach_session/recover` → لا إعادة attaching فعلية عبر الـ hook — `NONE` في `apple_avplayer.rs`; وسلوك AVPlayer نفسه `HYPOTHESIS`. | **لا observer داخلي** — `NONE`; واجهة عامة فقط `VideoPlayer::note_route_changed` — `src/player.rs:1235-1246`. `InterruptionReason::AudioFocusLoss` مُعرّف لكن غير مُصدر — `src/session.rs:65-76`, `session.rs:484` (اختبار فقط). توجيه AAudio في طبقة المنصة — `Cargo.toml:88-92`. | استطلاع cpal موجود — `audio.rs:371-421` — لكنه لا يُسلّح أي recovery لأن `session` غير مبني — `src/lib.rs:28-29`, `src/player/threaded.rs:218`. | **NONE** — لا audio module ولا observer — `src/lib.rs:4`, `web.rs`. |
| 8 | **Recovery hook: `close()`** | نعم: releases audio قبل reader — `windows.rs:1094-1103`. | نعم: `cancelReading` + حذف spill — `apple.rs:433-439`. | نعم — `apple.rs:433-439`; وAVPlayer يوقف الصوت ويحرر الـ claim — `apple_avplayer.rs:258-272`. | نعم: gate + تحرير codec/extractor عبر JNI — `android.rs:1337-1371`. | نعم: `State::Null` — `linux.rs:236-239`. | نعم: pause + destroy hls.js + revoke URL — `web.rs:437-448`. |
| 9 | **Recovery hook: `recover()`** | نعم: يعيد `setup_audio(false)` — `windows.rs:1078-1092`. | نعم: يعيد بناء الـ asset reader — `apple.rs:456-470`. | ملفات: نعم — `apple.rs:456-470`; AVPlayer HLS: **NONE** → `DEFAULT=true` — `decoder.rs:231-234`. | نعم: يمسح cooldown ويعيد المحاولة في القراءة — `android.rs:1329-1335`. | **NONE** — Hook مستبعد — `decoder.rs:231-234`; لا حلقة recovery — `player/threaded.rs:218`. | **NONE** — نفس السبب. |
| 10 | **Recovery hook: `attach_session()`** | نعم: `windows.rs:1069-1071` (مع `native-audio`). | نعم — `apple.rs:445-448`. | ملفات: نعم — `apple.rs:445-448`; AVPlayer HLS: **NONE**. | نعم — `android.rs:1324-1327`. | **NONE** — `decoder.rs:221-222`, `player.rs:934-935`. | **NONE** — نفس السبب. |
| 11 | **Recovery hook: `observe_shutdown()`** | **NONE** في `windows.rs` → `DEFAULT` no-op — `decoder.rs:204-212`; (HLS Stream نفسه يوفّره — `src/hls/mod.rs:1358-1360`). | نعم — `apple.rs:441-443`. | ملفات: نعم — `apple.rs:441-443`; AVPlayer نعم — `apple_avplayer.rs:224-226`. | نعم: StopFlag — `android.rs:1320-1322`, `android.rs:108-137`. | **NONE** — `decoder.rs:204-212`, `linux.rs`. | **NONE** — `web.rs`. |
| 12 | **Seek support** | `can_seek` من `MFMEDIASOURCE_CAN_SEEK` — `windows.rs:580-592`; `seek` يُرجع `UnsupportedOperation` إن لم يكن قابلًا — `windows.rs:887-907`. لكن `is_seekable()` يبقى `DEFAULT=true` — `decoder.rs:120-123` (تناقض — §3). | `open_at(position)` يعيد بناء reader — `apple.rs:413-415`, `apple.rs:172-199`; `is_seekable()` `DEFAULT=true` دون فحص — `decoder.rs:120-123`. | ملفات كما macOS؛ AVPlayer: `is_seekable=false` و`seek` دائمًا `UnsupportedOperation` — `apple_avplayer.rs:274-286`; HLS من ال playlist — `src/hls/mod.rs:444-445`, `hls/mod.rs:1192-1198`, `hls/mod.rs:1278-1290`. | `MediaExtractor.seekTo(SEEK_TO_PREVIOUS_SYNC)` + `codec.flush()` — `android.rs:166-180`, `android.rs:1292-1294`; `is_seekable` `DEFAULT=true` — `decoder.rs:120-123`. | `pipeline.seek_simple(FLUSH\|ACCURATE)` — `linux.rs:193-201`; `is_seekable` `DEFAULT=true` — `decoder.rs:120-123`. | `set_current_time` — `web.rs:365-370`; `is_seekable = !is_live()` — `web.rs:399-407`. |
| 13 | **Live support** | ملفات: `DEFAULT=false` — `decoder.rs:112-118`; HLS Stream يشتق `live = !media.ended` — `src/hls/mod.rs:434`, `hls/mod.rs:444`, `hls/mod.rs:1192-1194`. | مثل Windows — `decoder.rs:116`, `hls/mod.rs:434-445`. | AVPlayer HLS يُعلن `is_live()=true` **دائمًا** — `apple_avplayer.rs:274-276` — حتى لـ VOD (`HYPOTHESIS` الأثر على واجهة التطبيق). | `DEFAULT=false` — `decoder.rs:116`; HLS Stream `hls/mod.rs:434-445`. | `DEFAULT=false` — `decoder.rs:116`; HLS Stream `hls/mod.rs:434-445`. | `duration().is_infinite()` — `web.rs:399-403`. |
| 14 | **First-picture path** | أول عيّنة فيديو تحدد `origin` لكل الطوابع — `windows.rs:873-877`; صوت MF ينتظر أول صورة (`waiting`/`primed`) — `mf_audio.rs:296-307`, `mf_audio.rs:421-441`, `mf_audio.rs:1770-1791` — بسقف زمني `PRIME_TIMEOUT` — `mf_audio.rs:73-76`, `mf_audio.rs:1807-1827`. | pts من `presentation_time_stamp` مع fallback إلى `self.start` — `apple.rs:329-336`, `apple.rs:397-404`; reader يُفتح عند 0 — `apple.rs:103`. | ملفات كما macOS؛ AVPlayer ينتظر `ReadyToPlay` ≤10 s — `apple_avplayer.rs:45`, `apple_avplayer.rs:148-170` — ويحجب `read` حتى `hasNewPixelBufferForItemTime` — `apple_avplayer.rs:298-328` — وpts من `display_time` الحقيقي — `apple_avplayer.rs:362-375`. | codec يمرّ بـ `INFO_TRY_AGAIN_LATER` حتى أول صورة، بسقف `EMPTY_DEADLINE=30s` — `android.rs:42-45`, `android.rs:371-395`; pts = `presentationTimeUs` — `android.rs:861-865`, `android.rs:926-930`. | انتظار حالة `READY` ≤10 s — `linux.rs:28-32`, `linux.rs:102-106`; `try_pull_sample` — `linux.rs:250-264`; pts من `buffer.pts()` أو صفر — `linux.rs:329-332`. | `ready_state >= HAVE_CURRENT_DATA` وتغيّر `current_time` — `web.rs:31-33`, `web.rs:386-391`; pts = `element.current_time()` — `web.rs:343-349`; الحجم صفر حتى أول صورة → `VideoState::Loading` — `src/player.rs:947-950`. |
| 15 | **Known gaps (ملخّص)** | `set_audio_group` no-op — `decoder.rs:160`; `observe_shutdown` no-op — `decoder.rs:212`; مصادر `Bytes` يبقى الصوت هنا فقط — `audio.rs:615-623` + `windows.rs:1054-1062`; MF blocking read قد يؤخر `close()` — `HYPOTHESIS`. | HLS بـ TS غير قابل للقراءة — `mod.rs:107-110`; صوت TS صامت — `README.md:251`; لا recovery على مستوى النظام — `mod.rs:82-83`. | AVPlayer: `attach_session/recover` غائبان — `apple_avplayer.rs`; `is_live=true` و`is_seekable=false` مطلقًا — `apple_avplayer.rs:274-286`; مسار VOD عبر `open_hls` يمرّ بال AVPlayer — `player.rs:873-874`. | لا route observer داخلي — `player.rs:1235-1246`; `AudioFocusLoss` غير مُصدر — `session.rs:65-76`; ترتيب chroma يعتمد على الجهاز — `README.md:593`; `is_seekable` افتراضي غير محقّق — `decoder.rs:120-123`. | بلا `session` ولا recovery — `lib.rs:28-29`, `player/threaded.rs:218`; بلا decoder بدون feature — `unsupported.rs:9-16`; صوت TS صامت — `README.md:251`. | بلا audio/arbitration/session/hls modules — `lib.rs:4-29`; `hls.js` من CDN — `web.rs:38-39`; العنصر يبدأ `muted=true` — `web.rs:48-52`; `carries_audio=false` رغم ملكية الصوت — `decoder.rs:184`. |

---

## 2. (a) ميزات توجد على بعض المنصات فقط

1. **Observer تلقائي لتغيير مسار الصوت**: فقط iOS يثبّت `AVAudioSessionRouteChangeNotification` —
   `src/platform/ios_route.rs:22-43` عبر `src/platform/mod.rs:168-170`. Windows يكتشف التغيير
   داخل backend الصوت نفسه (`mf_audio.rs:1132-1196`)، macOS/Linux يعتمدان على استطلاع cpal
   العام (`audio.rs:371-421`)، Android ينتظر أن يستدعي المضيف `note_route_changed`
   (`player.rs:1235-1246`)، Web لا شيء (`lib.rs:4`).
2. **جلسة الاسترداد (Recovery session)**: موجودة على Windows/macOS/iOS/Android فقط —
   `src/lib.rs:28-29`, `src/session.rs:1-18`; غائبة على Linux وWeb، بما في ذلك واجهة
   `playback_state()` / `recovery_events()` (`src/player.rs:1265-1279`).
3. **Arbitration / audio groups**: الـ module موجود على كل المنصات عدا Web — `src/lib.rs:6-10` —
   لكنه **يُطبَّق فعليًا** فقط في المسار العام (`audio.rs:545-589`) وفي AVPlayer المباشر
   (`apple_avplayer.rs:228-296`). غائب عمليًا عن Windows MF وعن `hls::Stream` (الجدول 6/6b).
4. **مسار HLS الأصلي في المتصفح**: فقط Web يستخدم `canPlayType(HLS_TYPE)` ثم `hls.js` —
   `web.rs:35-39`, `web.rs:77-83`. iOS يستخدم AVPlayer (`mod.rs:121-128`)؛ البقية تضمّ
   المقاطع في ملف مؤقت (`hls/mod.rs:667-668`).
5. **صوت MPEG-TS فعلي**: Windows فقط — `README.md:240`, `README.md:251`. باقي المنصات
   يقرأ الصوت العام fMP4 فقط، فيبقى صوت TS صامتًا — `README.md:251`.
6. **Byte source مع صوت**: فقط Windows يملك مسارًا يقرأ المسارين من نفس ال reader للبايتات —
   `windows.rs:142-154`, `windows.rs:1054-1062`; المسار العام يتنحى عن البايتات —
   `audio.rs:615-623` — رغم أن Apple/Android/Linux يكتبون spill file للصورة فقط —
   `apple.rs:55-58`, `android.rs:978-983`, `linux.rs:36-40`.
7. **حالة live حقيقية من الوسيط**: Web (`web.rs:399-403`) وHLS Stream (`hls/mod.rs:434-445`)
   وAVPlayer (`apple_avplayer.rs:274-276`) فقط؛ بقية الـ decoders تستخدم `DEFAULT=false` —
   `decoder.rs:116`.
8. **`note_route_changed` عام لواجهة التطبيق**: مُعرّف فقط على non-Linux/non-wasm —
   `player.rs:1243-1246`.

## 3. (b) تناقضات خطيرة (Dangerous inconsistencies)

1. **`set_audio_group` صامت على Windows وعلى كل HLS**. `VideoPlayer::set_audio_group`
   يرسل أمرًا إلى المشغّل (`src/player.rs:1230-1233`, `src/player/threaded.rs:310`)، لكن
   `Reader` في Windows لا يعرّف ال hook → `DEFAULT` no-op — `src/decoder.rs:160` —
   و`hls::Stream` كذلك لا يعرّفه — `src/hls/mod.rs` (NONE). النتيجة: تجميع الصوت
   (mutual exclusion) الذي تعد به الوثيقة `src/player.rs:1219-1225` لا يعمل على هذه
   المسارات، بينما يعمل على المسار العام — `src/audio.rs:545-589`. **أخطر لأنه فشل صامت**.
2. **`is_seekable` يكذب على Windows/macOS/Android/Linux**. المشغّل يقرأ القيمة مرة عند
   الفتح (`src/player.rs:908-925`)، لكن هذه الـ decoders لا تتجاوز `DEFAULT=true` —
   `src/decoder.rs:120-123`. وفي Windows نفسه `seek` قد يُرجع `UnsupportedOperation` —
   `windows.rs:887-895` — رغم أن `is_seekable` قال true. على Linux/Android/macOS لا يوجد
   فحص قابلية عند الفتح إطلاقًا (`linux.rs:193-201`, `android.rs:166-180`,
   `apple.rs:413-415`).
3. **`carries_audio` على Web مضلّل**: العنصر يملك الصوت ويعرض `set_muted/volume/is_muted/volume`
   — `web.rs:421-435` — لكن القيمة تبقى `false` — `decoder.rs:184` — لأن وحدة `audio`
   غير مبنية — `src/lib.rs:4`. أي منطق تطبيق يتفرّع على `carries_audio()` سيُخطئ في
   وصف Web.
4. **iOS AVPlayer HLS يُعلن live=true / seekable=false بشكل غير مشروط** —
   `apple_avplayer.rs:274-286` — بما أن `platform::open` و`open_hls` يمرّران كل `.m3u8`
   إلى AVPlayer (`mod.rs:178-185`, `player.rs:873-874`)، فحتى VOD يُعرض كـ live. هذا
   يخالف HLS Stream الذي يشتق الحالة من `media.ended` — `hls/mod.rs:434-445`.
5. **إشارة تغيير المسار على iOS تصل session بلا مستقبل فعلي على مسار AVPlayer**:
   `route_changed_all()` يُسلّح recovery — `session.rs:189-201`, `threaded.rs:273-299` —
   لكن `AvPlayerDecoder` لا يعرّف `attach_session` ولا `recover` → `DEFAULT=true` —
   `decoder.rs:221-234`. هل يتعافى AVPlayer تلقائيًا؟ `HYPOTHESIS`؛ لا يوجد دليل كودي.
6. **`AudioFocusLoss` مُعرّف دون مُصدر**: `src/session.rs:65-76` يعدّه سببًا، لكن البحث
   في `src/` لا يجد استدعاءً لـ `interrupted(AudioFocusLoss)` خارج الاختبارات
   (`session.rs:484`). أي تطبيق يعتمد على هذا التصنيف على Android لن يراه.
7. **Windows `observe_shutdown` افتراضي no-op** — `decoder.rs:204-212` مقارنةً بـ
   `apple.rs:441-443`, `android.rs:1320-1322`, `hls/mod.rs:1358-1360`. مع مصادر URL
   شبكية قد يبقى `ReadSample` محجوبًا أثناء `VideoPlayer::close()` — `HYPOTHESIS`
   (الREADME يوصي بتجاوز ال hook — `README.md:462`).
8. **منطق أول صورة غير متماثل**: Windows ينتظر الصورة قبل إصدار الصوت
   (`mf_audio.rs:421-441`, `mf_audio.rs:1770-1791`)؛ المسار العام لا يفعل ذلك
   (`audio.rs:497-517` يزامن بالـ tolerance فقط)؛ Web يبدأ بصوت مكتوم
   (`web.rs:48-52`). لا يمكن للتطبيق افتراض محاذاة A/V متطابقة.
9. **Linux لديه استطلاع route لكنه بلا recovery**: `audio.rs:371-421` يعمل، لكن
   `lib.rs:28-29` و`player/threaded.rs:218` يستبعدان الـ session؛ النتيجة "اكتشاف
   بلا إصلاح"، وهو أسوأ من غياب الاكتشاف من حيث توقّع المستخدم.
10. **صوت الشريط المنفصل على Windows مشروط بزمن القراءة**: `hls::Stream::sync_audio`
    يتنحى إذا قال window decoder إنه يحمل الصوت — `hls/mod.rs:963-965` — بينما
    `carries_audio` في Windows يتغيّر بعد أول إعداد — `windows.rs:1061`, `windows.rs:718`.
    السلوك النهائي للـ separate rendition على Windows `HYPOTHESIS`.

## 4. (c) ما يجب على تطبيق cross-platform التعامل معه بنفسه

1. **لا تعتمد على audio groups كضمانة عابرة للمنصات**. إن أردت exclusivity حقيقية،
   طبّقها في طبقة التطبيق (أو افتح مصدرًا صوتيًا واحدًا فقط) لأن `set_audio_group` no-op
   على Windows MF وعلى HLS (`decoder.rs:160`, `hls/mod.rs` NONE) وغير موجود على Web
   (`lib.rs:6-10`).
2. **أبلغ عن تغيير المسار يدويًا على Android**: استدعِ `VideoPlayer::note_route_changed`
   من `AudioDeviceCallback`/`BroadcastReceiver` — `player.rs:1235-1246` — لأن لا observer داخلي
   (`ios_route` مقصور على iOS — `mod.rs:82-83`). على Linux/Web لا يوجد recovery إطلاقًا،
   فيجب على التطبيق إعادة فتح المشغّل عند فقدان الصوت (`lib.rs:28-29`).
3. **تعامل مع HLS بحذر**: استخدم `VideoPlayer::open_hls` للعناوين غير المنتهية بـ
   `.m3u8` — `player.rs:842-889`, `src/source.rs:61-77`; توقّع رفض Playlists المشفّرة
   (`README.md:220`); لا تتوقّع صوتًا من مقاطع MPEG-TS خارج Windows (`README.md:251`);
   وتذكّر أن Web يحتاج اتصالًا بـ CDN لتحميل `hls.js` أو متصفحًا يدعم HLS أصلًا
   (`web.rs:35-39`) مع CORS صحيح (`web.rs:57-62`).
4. **افتح الصوت على Web بعد تفاعل المستخدم**: العنصر يبدأ `muted=true` — `web.rs:48-52` —
   فيجب استدعاء `set_muted(false)` — `web.rs:421-423` — بعد gesture، وإلا بقي صامتًا.
5. **تحقّق من `is_seekable` ثم عالج `UnsupportedOperation` وقت التشغيل**: لأن القيمة
   افتراضية على أربع منصات (§3.2)، ولا يدعم AVPlayer الـ seek — `apple_avplayer.rs:282-286`
   — ولا HLS live — `hls/mod.rs:1286-1290` — ولا Windows عند غياب `MFMEDIASOURCE_CAN_SEEK`
   — `windows.rs:887-895`.
6. **لا تفترض أن live تعني الشيء ذاته**: iOS AVPlayer يقول live دائمًا
   (`apple_avplayer.rs:274-276`)؛ Web يقيس `Infinity` (`web.rs:399-403`)؛ HLS Stream
   يقيس `media.ended` (`hls/mod.rs:434-445`). وحدّد سياسة التطبيق لديك.
7. **راقب الحالة بنفسك على Linux/Web**: لا `playback_state()` ولا `recovery_events()`
   هناك (`player.rs:1265-1279` مقيّدة بـ cfg)، ولا حلقة استرداد (`threaded.rs:218`).
   استخدم `VideoState`/`events` وأعد الفتح عند الحاجة.
8. **قدّم صوتًا بديلًا لمصادر `Bytes` على غير Windows**: المسار العام يرفض البايتات —
   `audio.rs:615-623` — والـ spill file يُكتب للصورة فقط — `apple.rs:55-58`,
   `android.rs:978-983`, `linux.rs:36-40`. استخدم `Path`/`Url` إن أردت صوتًا مضمونًا.
9. **تحقّق من `is_supported()` و`decoder_name()` قبل الفتح**: `src/lib.rs:76-87`;
   Linux بدون `linux-gstreamer` يُرجع `VideoError::Unsupported` —
   `src/platform/unsupported.rs:12-15`, `Cargo.toml:37` — والحل هو `with_decoder`
   (`src/player.rs:891-906`, `README.md:462`).
10. **لا تتوقّع محاذاة A/V موحّدة**: راجع §3.8. إن كنت تبني مزامنة خارجية، استخدم
    `sync_clock` / `DecodedFrame.pts` — `src/decoder.rs:8-17`, `decoder.rs:188-195` —
    كمرجع، لا افتراض "الصوت يبدأ مع أول صورة" على كل المنصات.
11. **وفر decoder خاصًا للصيغ التي لا يقرأها النظام**: هذه هي واجهة الخروج الرسمية —
    `src/player.rs:891-906`, `README.md:462`; وهذا يشمل صوت TS على غير Windows وصيغًا
    يرفضها `createDecoderByType` على Android (`android.rs:1206-1214`) أو المتصفح على Web
    (`web.rs:41-47`).

---

## 5. ملاحظة ختامية حول الدقة

كل خلية في الجدول مستندة إلى سطر/مدى أسطر في ملف موجود بهذا المستودع، أو موسومة
`NONE`/`HYPOTHESIS`. لم تُستخدم أي أرقام أو نسب مئوية أو قياسات أداء من خارج الملفات
المذكورة. ما ورد تحت `HYPOTHESIS` هو استنتاج معلَن وليس حقيقة مُختبَرة على جهاز؛
وما ورد تحت `NONE` هو غياب تنفيذ في الكود لا حكم بعدم إمكانية قيام المنصة بذلك بنفسها.
