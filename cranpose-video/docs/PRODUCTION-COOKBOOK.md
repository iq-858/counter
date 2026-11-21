# دليل الإنتاج — وصفات `cranpose-video` العملية

هذا الملف دليل تطبيق لا دليل مرجع. كل وصفة تبدأ بهدف، ثم ترتيب استدعاءات `API`، ثم مخطط كود مأخوذ من التوقيعات الموجودة في المصدر، ثم الفخاخ، ثم فصل واضح بين ما هو مُتحقَّق منه في المصدر وما هو فرضية تحتاج جهازاً.

## وسوم التوثيق

| الوسم | المعنى |
| --- | --- |
| `[SOURCE-VERIFIED]` | حقيقة مقروءة من الملف المُشار إليه، أو محمية باختبار تكامل في `tests/`. ليست قياس جهاز. |
| `[HYPOTHESIS]` | نتيجة متوقعة في وقت التشغيل أو على الجهاز، لم يُلتقط لها دليل هنا. |
| `[GAP]` | شيء غير موجود في هذه الشجرة. لا يُنسب للمكتبة. |
| `[REPORT-ONLY]` | رقم أو ادعاء موروث من تقرير قديم، بلا أثر قابل للتحقق في هذه الشجرة. لا يُستخدم كرقم إنتاج. |

المرجع الأعلى هو المصدر. حين يختلف `README.md` عن الكود، يُؤخذ الكود. مثال: `README.md:588` يقول إن `read_nv12` الافتراضي يُعيد `Ok(None)`، بينما `src/decoder.rs:74-79` يُعيد `VideoError::UnsupportedOperation`؛ المصدر هو الصحيح هنا.

الأرقام الظاهرة داخل مخططات الكود، مثل أرقام المجموعات وفترات الاقتراع، قيم اختيارها المتصل لتكون الأمثلة قابلة للقراءة. ليست ثوابت للمكتبة. الثوابت المذكورة كحقائق هي فقط تلك المقتبسة من المصدر مع رقم الملف والسطر.

## قواعد قبل كل الوصفات

1. كل `VideoPlayer` يملك `slot` و`engine` مستقلين. لا يوجد مُفكِّك singleton. يُنشأ الاثنان في `src/player.rs:936-943`. المكتبة تسمح بعدة مشغّلات في آن واحد، كما يظهر في `tests/audio_group.rs:125-128`.
2. `VideoPlayer::close(&mut self)` يرفع علماً أحادي الاتجاه ويرسل `Command::Shutdown` ويفصل خيط العمل بدل انتظاره (`src/player.rs:1372-1411`, `src/player/threaded.rs:111-121`). `Drop` يستدعي `close` (`src/player.rs:1431-1435`). لا تُنتظر قراءة محجوبة بعد الإغلاق.
3. `set_audio_group` يرسل أمراً إلى خيط فك الترميز عبر قناة واحدة (`src/player/threaded.rs:104-106`)، فالترتيب بينه وبين `play` هو ترتيب الاستدعاء على الخيط نفسه.
4. المطالبة بملكية الصوت تحدث عند أول `play()`، لا عند `open` (`src/audio.rs:424-434`, `src/audio.rs:569-574`). هذا هو الأساس الذي يجعل التحميل المسبق لا يُكتم المشغّل الحالي.
5. مساعد مشترك لتقديم الإطارات، سيُستدعى في الوصفات:

```rust,no_run
use cranpose_video::VideoPlayer;

fn upload(_: u32, _: u32, _: &[u8]) {}
fn present() {}

fn pump(player: &VideoPlayer) {
    if let Some(frame) = player.take_frame() {              // src/player.rs:1324
        upload(frame.width(), frame.height(), frame.rgba()); // src/frame.rs:177,223
        present();
        player.recycle(frame);                               // src/player.rs:1333
    }
}
```

`take_frame` يُعيد أحدث صورة منشورة فقط، و`recycle` يُعيد مخزن البكسل إلى المسبح. إسقاط الإطار بدل إعادة تدويره يعمل أيضاً لكنه يفقد المخزن (`src/player.rs:1316-1335`).

## الوصفة 1 — خلاصة Reels بمشغّل نشط واحد

### الهدف

عرض مقطع واحد في كل لحظة في خلاصة تمريرية، مع مشغّل واحد ومجموعة صوت افتراضية `0`، بحيث يكتم المقطع الأقدم تلقائياً عند انتقال المشغّل الأحدث إلى `play`.

### استدعاءات API بالترتيب

1. `VideoWithControls(source, modifier)` (`src/widget.rs:181-183`).
2. داخلياً: `rememberVideoPlayer(source)` مفتاحه `source` عبر `rememberKeyed` (`src/widget.rs:157-165`).
3. داخلياً: `picture(...)` يستدعي `player.play()` ثم يضخ الإطارات (`src/widget.rs:405-438`).
4. عند تغيير المقطع: تغيير `source` يُسقط المشغّل القديم ويغلق خيوطه (`src/widget.rs:159-164`).

### مخطط الكود

```rust,no_run
use cranpose_ui::Modifier;
use cranpose_video::VideoWithControls;

#[composable]
fn ReelsScreen(clip: &'static str) {
    // استدعاء واحد = مشغّل واحد. لا تستدعِ rememberVideoPlayer هنا أيضاً.
    VideoWithControls(clip, Modifier::empty().fill_max_size());
}
```

### الفخاخ

- `VideoPlayerHandle` لا يعرض `set_audio_group` ولا `take_frame` ولا `close`. هذا مذكور صراحة في `docs/FLEET-FULL-REVIEW-2026-11-21.md:30`، وواجهة المقبض في `src/widget.rs:39-150` تثبت الغياب. إذا احتاج التطبيق هذه العمليات، يملك `VideoPlayer` مباشرة.
- استدعاء `rememberVideoPlayer(source)` و`VideoWithControls(source, ...)` في نفس الشاشة ينشئ مشغّلين منفصلين؛ لكل واحد موقع تركيب مختلف.
- `picture()` يستدعي `play()` بنفسه (`src/widget.rs:421`)، فلا حاجة لاستدعاء إضافي من التطبيق. كما أنه يتوقف عند إغلاق المشغّل بدل متابعة الاقتراع (`src/widget.rs:424-428`).

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` المجموعة الافتراضية `0`، والأحدث مطالباً فيها يُكتم الأقدم (`src/arbitration.rs:22-23`, `src/arbitration.rs:43-66`).
- `[SOURCE-VERIFIED]` المطالبة عند `play` لا عند `open` (`src/audio.rs:432-434`)؛ اختبار `tests/audio_group.rs:121-153` يغطي مشغّلين في المجموعة نفسها.
- `[SOURCE-VERIFIED]` المقبض مفتاحه `source` (`src/widget.rs:163-164`)؛ التعليق في `src/widget.rs:159-162` يذكر حالة Reels صراحة.
- `[HYPOTHESIS]` سلوك التمرير نفسه على جهاز حقيقي، ومدة بقاء الإطار القديم أثناء الانتقال، لم يُقَسا هنا.

## الوصفة 2 — فيديوان جنباً إلى جنب وكلاهما بالصوت

### الهدف

تشغيل مقطعين في الوقت نفسه مع بقاء صوتيهما مسموعين، عبر تعيين مجموعة صوت مختلفة لكل مشغّل.

### استدعاءات API بالترتيب

1. `VideoPlayer::open(left)` و`VideoPlayer::open(right)` (`src/player.rs:833`).
2. `left.set_audio_group(group_a)` ثم `right.set_audio_group(group_b)` (`src/player.rs:1230`).
3. `left.play()` و`right.play()` (`src/player.rs:1033`) — المطالبة تحدث هنا.
4. ضخ الإطارات لكل مشغّل عبر `pump` في حلقة الرسم.

### مخطط الكود

```rust,no_run
use cranpose_video::{VideoError, VideoPlayer};

struct Split {
    left: VideoPlayer,
    right: VideoPlayer,
}

fn open_split(left_path: &str, right_path: &str) -> Result<Split, VideoError> {
    let mut left = VideoPlayer::open(left_path)?;
    let mut right = VideoPlayer::open(right_path)?;

    // group 0 هو الافتراضي. مجموعتان مختلفتان تحفظان صوتين مستقلين.
    left.set_audio_group(1)?;                    // src/player.rs:1230
    right.set_audio_group(2)?;                   // src/player.rs:1230

    left.play()?;                                // src/player.rs:1033
    right.play()?;
    Ok(Split { left, right })
}

fn draw(split: &Split) {
    pump(&split.left);
    pump(&split.right);
}

fn close(split: &mut Split) {
    split.left.close();                          // src/player.rs:1378
    split.right.close();
}
```

لا يمكن بناء هذه الوصفة على `VideoPlayerHandle` لأنه لا يعرض `set_audio_group` (`src/widget.rs:39-150`)؛ لذلك يملك الكود المشغّلين مباشرة ويرسم إطاراتهما بنفسه.

### الفخاخ

- تعيين المجموعة نفسهما للمشغّلين يجعل الأحدث يكتم الأقدم حسب التصميم (`src/arbitration.rs:3-10`). هذا ليس عطلاً.
- `set_audio_group` يقبل `u64` ولا يتحقق من المدى (`src/player.rs:1230-1233`). التطبيق يختار الأرقام ويتجنب تكرارها بين مجموعات مختلفة.
- استدعاء `set_audio_group` بعد `close` يُعيد `VideoError::Closed` (`tests/audio_group.rs:180-193`).
- بدون تفعيل `native-audio`، المكتبة تسجّل حالة الصوت فقط ولا تشغّل صوت المصدر؛ انظر `Cargo.toml:38-42` و`README.md:230`. على الويب يملك عنصر `<video>` الصوت ولا أثر لتفعيل الميزة (`Cargo.toml:40-42`).
- مصادر `Bytes` وHLS لا تُغلَّف بمسار الصوت المزدوج (`src/audio.rs:611-619`)؛ إذا كان المقطع بايتات معبّأة، الصوت مسؤولية التطبيق أو مُفكِّك مخصص.

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` مجموعات مختلفة تعمل معاً، ومجموعة واحدة حصرية، ومطالبة واحدة تُرسل مرة واحدة إلى الخطاف؛ `tests/audio_group.rs:97-178` يغطي الحالات الثلاث.
- `[SOURCE-VERIFIED]` المطالبة عند أول استخدام مسموع، مع تعليق صريح بأن التحميل المسبق لا يُكتم الحالي (`src/audio.rs:432-434`).
- `[SOURCE-VERIFIED]` إغلاق المشغّل يُحرِّر مطالبته فيُرفع الأقدم في المجموعة (`src/audio.rs:336-348`, `src/arbitration.rs:68-84`).
- `[HYPOTHESIS]` أن يظهر الصوتان فعلياً على جهاز مع تفعيل `native-audio`؛ لا يوجد تشغيل صوت مُلتقط على جهاز في هذه الشجرة.

## الوصفة 3 — مشغّل تراكب / Picture-in-picture

### الهدف

رسم مشغّل صغير فوق محتوى رئيسي داخل نافذة Cranpose واحدة.

### استدعاءات API بالترتيب

1. `Video(main, Modifier::empty().fill_max_size())` للمشغّل الرئيسي (`src/widget.rs:172-175`).
2. `Video(overlay, modifier)` ذو `width`/`height`/`offset` (`src/widget.rs:172-175`, نمط `Modifier` في `src/widget.rs:375-399`).
3. اختياري: `set_muted(true)` للراكب عبر `VideoPlayerHandle` (`src/widget.rs:92-94`)؛ أو مجموعتان مختلفتان كما في الوصفة 2.

### مخطط الكود

```rust,no_run
use cranpose_ui::Modifier;
use cranpose_video::Video;

#[composable]
fn PlayerWithOverlay(
    main: &'static str,
    overlay: &'static str,
    width: f32,
    height: f32,
    x: f32,
    y: f32,
) {
    Video(main, Modifier::empty().fill_max_size());
    Video(
        overlay,
        Modifier::empty().width(width).height(height).offset(x, y),
    );
}
```

القيم `width`/`height`/`x`/`y` تُمرَّر من تخطيط التطبيق. لا أرقام ثابتة مفروضة هنا.

### الفخاخ

- `Video` يستدعي `rememberVideoPlayer` داخلياً (`src/widget.rs:174`)، لكل استدعاء مشغّل مستقل. مشغّلان يعني خيطي فك ترميز ومخزنين.
- إذا كان الراكب يحتاج إلى `set_audio_group`، استخدم الوصفة 2؛ المقبض لا يعرض المجموعة (`src/widget.rs:39-150`).
- لا يعني "تراكب" في هذه الوصفة وضع النظام `Picture-in-Picture` على Android أو iOS. المكتبة لا تملك واجهة لوضع النظام؛ `[GAP]` لم يُذكر في التصديرات العامة (`src/lib.rs:45-66`).
- إخفاء الراكب لا يُوقف فك التركيب تلقائياً؛ التطبيق يزيل التركيب (أو يغلق المشغّل الذي يملكه) عند الإخفاء.

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]`: `Video` مجرد غلاف حول `picture(rememberVideoPlayer(source), modifier)` (`src/widget.rs:172-175`)؛ الرسم يحدث داخل الصندوق الذي يعطيه `modifier`.
- `[SOURCE-VERIFIED]` عدة مشغّلات متزامنة مسموح بها (`src/player.rs:936-943`, `tests/audio_group.rs:125-128`).
- `[HYPOTHESIS]` سلوك النظام عند وضع التطبيق في PiP الحقيقي، أو عند تداخل النوافذ، خارج نطاق هذه الشجرة.

## الوصفة 4 — HLS مباشر مع ظهور الأخطاء

### الهدف

فتح قناة حية والسماح لشاشة التشخيص برؤية ما تبتلعه المكتبة من أخطاء بدلاً من الاكتفاء بحالة `Playing`.

### استدعاءات API بالترتيب

1. `set_diagnostic_sink(Some(Box::new(...)))` (`src/diagnostics.rs:37`) أو `start_log_file(path)` (`src/logfile.rs:65`).
2. `VideoPlayer::open_hls(url)` (`src/player.rs:854`) — يتطلب `VideoSource::Url` (`src/player.rs:865-869`).
3. `player.is_live()` و`player.duration()` و`player.is_seekable()` (`src/player.rs:1005-1012`).
4. `player.play()` (`src/player.rs:1033`).
5. في الحلقة: `pump`، ثم `player.state()` (`src/player.rs:982`)، ثم `player.recovery_events()` (`src/player.rs:1277`) و`player.events()` (`src/player.rs:1287`).
6. `player.close()` (`src/player.rs:1378`) ثم `set_diagnostic_sink(None)`.

### مخطط الكود

```rust,no_run
use std::time::{Duration, Instant};
use cranpose_video::{
    SessionEvent, VideoError, VideoPlayer, VideoState, set_diagnostic_sink,
};

fn watch_live(url: &str, budget: Duration) -> Result<(), VideoError> {
    set_diagnostic_sink(Some(Box::new(|level, target, message| {
        eprintln!("{level} {target}: {message}");                 // src/diagnostics.rs:37
    })));
    let mut player = VideoPlayer::open_hls(url)?;                 // src/player.rs:854
    debug_assert!(player.is_live());                              // src/player.rs:1005
    debug_assert_eq!(player.duration(), None);                    // src/player.rs:995
    debug_assert!(!player.is_seekable());                         // src/player.rs:1010
    player.play()?;                                               // src/player.rs:1033

    let deadline = Instant::now() + budget;
    let mut last_sequence = 0u64;
    while Instant::now() < deadline {
        pump(&player);

        match player.state() {                                    // src/player.rs:982
            VideoState::Failed(error) => {
                let detail = error.detail().unwrap_or("<no backend detail>");
                eprintln!("playback failed: {error:?} detail={detail}"); // src/error.rs:38
                break;
            }
            VideoState::Ended => break,
            _ => {}
        }

        // سجل الاستعادة منفصل عن سجل الأحداث، وكلاهما لا يُفرَّغ عند القراءة.
        for event in player.recovery_events() {                   // src/player.rs:1277
            match event {
                SessionEvent::Failed { stage, reason } => eprintln!("recovery failed at {stage}: {reason}"),
                SessionEvent::RouteChanged => eprintln!("output route changed"),
                SessionEvent::Recovered { attempt } => eprintln!("sound recovered on attempt {attempt}"),
                _ => {}
            }
        }
        for record in player.events() {                           // src/player.rs:1287
            if record.sequence() > last_sequence {
                last_sequence = record.sequence();                // src/telemetry.rs:153
                eprintln!("#{} {:?}", record.sequence(), record.event());
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }

    let snapshot = player.diagnostics();                          // src/player.rs:1312
    eprintln!("frames={} reanchors={} errors={} live={}",
        snapshot.frames_published(), snapshot.reanchors(),
        snapshot.errors(), snapshot.is_live());
    player.close();                                               // src/player.rs:1378
    set_diagnostic_sink(None);                                    // src/diagnostics.rs:37
    Ok(())
}
```

### الفخاخ

- `start_log_file` يستبدل أي مصدر تشخيصي مثبّت بـ `set_diagnostic_sink`؛ لا يعملان معاً في الوقت نفسه (`README.md:425`).
- رد النداء يعمل على الخيط الذي كشف الشرط، غالباً خيط فك الترميز؛ يجب أن يكون سريعاً وآمناً للخيوط (`src/diagnostics.rs:10-13`).
- التصفية تتم في نقطة واحدة قبل رد النداء: بيانات `URL` تُجرّد من الرمز والاستعلام (`src/diagnostics.rs:47-51`, `src/diagnostics.rs:60-98`). لا تُرسل النص الخام إلى سجل خارجي متجاوزاً هذه النقطة.
- على iOS، `open_hls` يُمرِّر عنوان القائمة الأصلي إلى `AVPlayer` بدل مُفكِّك الحزم داخل المكتبة (`src/player.rs:870-874`, `src/platform/mod.rs:118-128`).
- القنوات المشفّرة بـ `#EXT-X-KEY` تُرفض عند الفتح بـ `UnsupportedOperation` (`README.md:220`).
- لا يوجد `seek_to_live` عام؛ `[GAP]` في التصديرات (`src/lib.rs:45-66`)، ومذكور في `docs/FLEET-FULL-REVIEW-2026-11-21.md:31`.

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` `open_hls` يتخطى فحص اللاحقة وغلّاف الصوت المباشر، ويقرأ القائمة على خيط فك الترميز (`src/player.rs:842-889`).
- `[SOURCE-VERIFIED]` مراحل الفشل الحقيقية هي `hls.open` و`hls.window` (`src/hls/mod.rs:1465-1487`)، وتُرسل إلى الجلسة عبر `session.failed` (`src/hls/mod.rs:1437-1450`).
- `[SOURCE-VERIFIED]` الفشل يصل مرتين: قيمة مُعادة من الاستدعاء ونسخة في `VideoState::Failed` (`src/error.rs:1-6`, `tests/observability.rs:242-260`).
- `[SOURCE-VERIFIED]` `PlaybackDiagnostics` يجمع العدّادات نفسها في لقطة واحدة (`src/telemetry.rs:359-495`).
- `[HYPOTHESIS]` أن تبقى القناة حية بلا توقف على شبكة حقيقية؛ النقل والتكيّف مُوصوفان في `README.md:194-224` لكنهما غير مقيسين على جهاز هنا.

## الوصفة 5 — مُفكِّك `VideoDecoder` مخصص لبايتات معبّأة أو ترميز غير مدعوم

### الهدف

قراءة صيغة لا يعرفها مُفكِّك المنصة، مع إبقاء واجهة `VideoPlayer` كما هي.

### استدعاءات API بالترتيب

1. تنفيذ `VideoDecoder::info` و`seek` و`read` (`src/decoder.rs:29-46`).
2. تنفيذ اختياري: `observe_shutdown` (`src/decoder.rs:204-212`)، `close` (`src/decoder.rs:197-202`)، `carries_audio` (`src/decoder.rs:176-186`).
3. `VideoPlayer::with_decoder(decoder)` (`src/player.rs:901`).
4. `play` ثم `take_frame`/`recycle` كما في الوصفة 1.

### مخطط الكود

```rust,no_run
use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::Duration,
};
use cranpose_video::{
    DecodedFrame, FrameRate, VideoDecoder, VideoError, VideoInfo, VideoPlayer,
};

/// بايتات RGBA معبّأة بإطارات متتالية بلا حشو، محفوظة داخل الحزمة.
struct PackagedDecoder {
    bytes: &'static [u8],
    width: u32,
    height: u32,
    rate: FrameRate,
    next: usize,
    done: bool,
    shutdown: Arc<AtomicBool>,
}

impl PackagedDecoder {
    fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4       // src/frame.rs:5-8
    }
}

impl VideoDecoder for PackagedDecoder {
    fn info(&self) -> VideoInfo {
        let frames = (self.bytes.len() / self.frame_bytes()) as u32;
        let duration = self.rate.frame_duration()             // src/info.rs:20-23
            .map(|per_frame| per_frame.saturating_mul(frames));
        VideoInfo {
            width: self.width,
            height: self.height,
            duration,
            frame_rate: Some(self.rate),
            ..VideoInfo::default()
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        let step = self.rate.frame_duration().ok_or(VideoError::UnsupportedOperation {
            operation: "seeking a source without a frame rate",
        })?;
        let index = position.as_nanos().checked_div(step.as_nanos()).unwrap_or(0)
            as usize;
        self.next = index.saturating_mul(self.frame_bytes()).min(self.bytes.len());
        self.done = false;
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        if self.done || self.shutdown.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let size = self.frame_bytes();
        let end = self.next.saturating_add(size).min(self.bytes.len());
        if self.next >= end {
            self.done = true;
            return Ok(None);                                    // نهاية الوسيط
        }
        rgba.clear();
        rgba.extend_from_slice(&self.bytes[self.next..end]);    // src/decoder.rs:41-43
        let index = self.next / size;
        let pts = self.rate.frame_duration()
            .map(|per_frame| per_frame.saturating_mul(index as u32))
            .unwrap_or_default();
        self.next = end;
        Ok(Some(DecodedFrame {                                  // src/decoder.rs:8-17
            pts, width: self.width, height: self.height,
        }))
    }

    fn observe_shutdown(&mut self, flag: Arc<AtomicBool>) {
        self.shutdown = flag;                                   // src/decoder.rs:204-212
    }
}

fn open(decoder: PackagedDecoder) -> Result<VideoPlayer, VideoError> {
    VideoPlayer::with_decoder(decoder)                          // src/player.rs:901
}
```

### الفخاخ

- لا تُعلن `PixelFormat::Nv12` من `output_format` دون تنفيذ `read_nv12`؛ الافتراضي يُعيد `UnsupportedOperation`، والمشغّل يعود إلى `read` (`src/decoder.rs:54-79`). هذا يختلف عمّا كُتب قديماً في `README.md:588`.
- `read` يجب أن يضبط حجم `rgba` إلى `width * height * 4` (`src/decoder.rs:41-43`). البكسلات الناقصة تُرفض قبل أن تصل إلى المشغّل (`tests/frame_matrix.rs:63-95`).
- `with_decoder` لا يضيف مسار صوت؛ الكود الذي يمرّر مُفكِّكاً يبنى بلا صوت ما لم يكن المُفكِّك نفسه يحمل الصوت ويُعلن `carries_audio() -> true` (`src/player.rs:901-906`, `src/decoder.rs:176-186`).
- مُفكِّك يحجب في `read` يتحقق من علم الإغلاق الذي يأتي عبر `observe_shutdown`؛ وإلا قد يبقى الخيط منفصلاً بعد `close` (`src/decoder.rs:204-212`, `src/player.rs:1372-1404`).
- `seek` على بث لا يُطلب منه الرجوع يُعيد `VideoError::UnsupportedOperation` (`src/decoder.rs:33-39`). المشغّل يسأل مرة عند الفتح عبر `is_seekable` و`is_live` (`src/player.rs:911-914`).

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` ثلاث طرق إلزامية فقط، وكل الخطافات الأخرى ذات قيم افتراضية (`src/decoder.rs:29-235`).
- `[SOURCE-VERIFIED]` المشغّل يمرّر علم الإغلاق والجلسة إلى المُفكِّك قبل نقله إلى خيط العمل (`src/player.rs:927-935`).
- `[SOURCE-VERIFIED]` `close` للمُفكِّك يُستدعى على خيط فك الترميز قبل إسقاطه (`src/decoder.rs:197-202`).
- `[SOURCE-VERIFIED]` مثال كامل على مُفكِّك مخصص في `README.md:464-539`.
- `[HYPOTHESIS]` أن يقرأ مُفكِّك الترميز غير العادي بيانات حقيقية بالشكل المفترض هنا؛ التنسيق نفسه خارج هذه الشجرة.

## الوصفة 6 — تحميل المقطع التالي مسبقاً بلا إكمات الحالي

### الهدف

تجهيز مشغّل المقطع التالي قبل التمرير إليه، من غير أن يُكتم صوت المقطع الحالي.

### استدعاءات API بالترتيب

1. `VideoPlayer::open(next_path)` للمقطع التالي، من دون `play` (`src/player.rs:833`).
2. اختياري: `next.take_frame()` لتحضير أول صورة (`src/player.rs:1324`, `src/player.rs:1318-1320`).
3. عند التمرير: `current.pause()` (`src/player.rs:1059`).
4. `next.play()` (`src/player.rs:1033`) — هنا تحدث المطالبة بالمجموعة.
5. `current.close()` (`src/player.rs:1378`) — هنا تُحرَّر المطالبة القديمة.

### مخطط الكود

```rust,no_run
use cranpose_video::{VideoError, VideoPlayer};

fn preload(path: &str) -> Result<VideoPlayer, VideoError> {
    let next = VideoPlayer::open(path)?;   // بلا play: لا مطالبة بمجموعة الصوت
    let _ = next.take_frame();             // أول صورة متاحة أثناء الإيقاف
    Ok(next)
}

fn advance(current: &mut VideoPlayer, next: VideoPlayer) -> Result<(), VideoError> {
    let _ = current.pause();                                    // src/player.rs:1059
    let mut next = next;
    next.play()?;                                               // src/player.rs:1033
    current.close();                                            // src/player.rs:1378
    *current = next;
    Ok(())
}
```

### الفخاخ

- استدعاء `play()` على المقطع المحمّل مسبقاً قبل التمرير إليه يجعله يطالب بالمجموعة ويُكتم الحالي (`src/audio.rs:432-434`, `src/audio.rs:569-574`).
- `take_frame` على مشغّل متوقف يُعيد صورة واحدة بعد الفتح وبعد كل `seek` (`src/player.rs:1318-1320`)؛ لا يعني ذلك أن المقطع يبدأ بالتشغيل.
- إغلاق المشغّل القديم يُحرِّر مطالبته ويسمح برفع مطالبة سابقة في المجموعة نفسها (`src/audio.rs:336-348`, `src/arbitration.rs:68-84`). تأخير الإغلاق يترك مطالبتين في السجل.
- لا يمكن تحقيق هذا النمط عبر `VideoPlayerHandle`؛ المقبض لا يعرض `close` (`src/widget.rs:39-150`).
- المشغّل المحمّل مسبقاً يبقى مفتوحاً ويحتفظ بخيطه ومخزنه؛ التطبيق الذي يحمّل عدة مقاطع دفعة واحدة يتحمل عدداً مناسباً من الخيوط. `[SOURCE-VERIFIED]` كل مشغّل يملك `engine` مستقلاً (`src/player.rs:936-943`).

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` التعليق الصريح: المطالبة عند أول استخدام مسموع حتى لا يُكتم التحميل المسبق المقطع الحالي (`src/audio.rs:432-434`).
- `[SOURCE-VERIFIED]` `arbitrate` لا يطالب أبداً؛ `sync_clock` يستدعيها على كل إطار منشور، بما في ذلك الإطارات المنشورة أثناء الإيقاف (`src/audio.rs:541-565`).
- `[SOURCE-VERIFIED]` `set_group` يُحرِّر المطالبة القديمة قبل مطالبة جديدة (`src/audio.rs:579-592`).
- `[HYPOTHESIS]` أن يظهر الانتقال بلا انقطاع مسموع على جهاز حقيقي؛ لم يُلتقط تسجيل صوتي هنا.

## الوصفة 7 — فقدان تركيز الصوت وتغيّر المسار على Android وiOS

### الهدف

الفصل بين ما تفعله المكتبة تلقائياً وما يجب على التطبيق فعله عند خروج سماعة أو اتصال Bluetooth أو فقدان تركيز الصوت.

### ما تفعله المكتبة

- على iOS، أول استدعاء لـ `VideoPlayer::open` يثبّت مراقب `AVAudioSessionRouteChangeNotification` مرة واحدة لكل عملية (`src/platform/mod.rs:167-170`, `src/platform/ios_route.rs:22-43`).
- المراقب يستدعي `PlaybackSession::route_changed_all()` (`src/platform/ios_route.rs:28-30`)، الذي يُسلّح محاولة تحقق واحدة على كل جلسة حية (`src/session.rs:192-201`).
- `route_changed()` يضيف `SessionEvent::RouteChanged` ويضبط موعد إعادة المحاولة إذا لم يكن مضبوطاً (`src/session.rs:278-293`).
- جميع أعمال إعادة وصل الصوت تحدث على خيط فك الترميز (`src/decoder.rs:224-234`, `src/player.rs:931-935`).

### ما لا تفعله المكتبة

- `[GAP]` لا يوجد مستدعٍ إنتاجي لـ `route_changed()` على Android؛ الاستخدام في هذه الشجرة في الاختبارات فقط، كما هو موثّق في `docs/ANDROID-SUPPORT-FLEET-REVIEW.md:23` و`docs/FIX-SEQUENCING.md:190`.
- `[GAP]` `InterruptionReason::AudioFocusLoss` مُعرَّف (`src/session.rs:64-76`) لكن بلا مستدعٍ في المصدر؛ تم البحث في `src/` ولم يُوجد استدعاء لـ `interrupted(...)` خارج اختبار الوحدة في `src/session.rs:484`.
- `[GAP]` لا تطلب المكتبة `AudioFocusRequest` ولا تتعامل مع `AUDIOFOCUS_GAIN`؛ هذا من مسؤوليات التطبيق (`docs/ANDROID-SUPPORT-FLEET-REVIEW.md:14,22`).
- `[GAP]` لا توجد طريقة عامة على `VideoPlayer` لتسجيل فقدان التركيز كـ `InterruptionReason::AudioFocusLoss`؛ التصديرات في `src/lib.rs:54-58` وواجهة المشغّل في `src/player.rs:1235-1246` لا تتضمنها. مُفكِّك مخصص يستطيع الوصول إلى الجلسة عبر `attach_session` (`src/decoder.rs:214-222`) واستدعاء `interrupted` (`src/session.rs:258-270`).

### استدعاءات API بالترتيب (جانب التطبيق)

1. مستمع جهاز الإخراج يستدعي `player.note_route_changed()` (`src/player.rs:1244`).
2. مستمع تركيز الصوت يوقف أو يستأنف المشغّل بنفسه عبر `pause`/`play` (`src/player.rs:1059`, `src/player.rs:1033`).
3. شاشة التشخيص تقرأ `player.playback_state()` و`player.recovery_events()` (`src/player.rs:1266`, `src/player.rs:1277`).

### مخطط الكود

```rust,no_run
use cranpose_video::{PlaybackState, SessionEvent, VideoError, VideoPlayer};

fn on_output_device_changed(player: &VideoPlayer) {
    // غير متاح على Linux أو wasm: src/player.rs:1243
    #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
    player.note_route_changed();                                // src/player.rs:1244
    #[cfg(any(target_os = "linux", target_arch = "wasm32"))]
    let _ = player;
}

fn on_focus_loss_permanent(player: &VideoPlayer) -> Result<(), VideoError> {
    player.pause()                                              // src/player.rs:1059
}

fn on_focus_gain(player: &VideoPlayer) -> Result<(), VideoError> {
    player.play()                                               // src/player.rs:1033
}

fn show_audio_state(player: &VideoPlayer) {
    let state = player.playback_state();                        // src/player.rs:1266
    if state == PlaybackState::Recovering {
        eprintln!("picture continues, sound is coming back");
    }
    for event in player.recovery_events() {                     // src/player.rs:1277
        if let SessionEvent::Failed { stage, reason } = event {
            eprintln!("{stage}: {reason}");
        }
    }
}
```

### الفخاخ

- `note_route_changed` مُرَّز بـ `cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))` (`src/player.rs:1243`). لا تستدعِه من كود مشترك مع Linux أو الويب دون `cfg`.
- كل استدعاء لـ `route_changed()` يضيف حدثاً حتى لو كان `retry_at` مضبوطاً مسبقاً (`src/session.rs:288-292`). على التطبيق تجميع الإشارات المتقاربة؛ التوصية موثّقة في `docs/FIX-SEQUENCING.md:194`.
- تغيّر المسار يُسلّح محاولة فقط ولا يرسل أمر إيقاظ إلى خيط نائم (`docs/MOBILE-RISK-REGISTER.md:142-150`). إذا كان المشغّل متوقفاً، تأخّر التحقق حتى يصل أمر أو إطار آخر.
- `Interrupted` لا يعني أن الصورة توقفت؛ الصورة تبقى وتُحدَّث أثناء استعادة الصوت (`tests/audio_route_switch.rs:235-265`).
- سجل الاستعادة محدود بـ 64 حدثاً و`Closed` دائماً أخير (`src/session.rs:44-46`, `src/session.rs:434-450`, `tests/audio_route_switch.rs:298-321`).

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` مراقب iOS يُثبَّت مرة واحدة ويستدعي `route_changed_all` (`src/platform/ios_route.rs:22-43`).
- `[SOURCE-VERIFIED]` `route_changed()` لا يُرسل أي `Command`؛ يُسلّح `retry_at` فقط (`src/session.rs:278-293`, `docs/MOBILE-RISK-REGISTER.md:142-150`).
- `[SOURCE-VERIFIED]` غياب مستدعٍ Android لكل من مسار الصوت وفقدان التركيز (`docs/ANDROID-SUPPORT-FLEET-REVIEW.md:21-23`، وبحث المصدر).
- `[HYPOTHESIS]` أن يؤدي استدعاء التطبيق لـ `note_route_changed()` من مستمع النظام إلى عودة الصوت فعلياً؛ لم يُختبر على جهاز.
- `[HYPOTHESIS]` أن يمنع طلب تركيز الصوت من التطبيق قتل المشغّل أثناء تشغيل آخر؛ سياسة النظام وتطبيق التطبيق خارج هذه الشجرة.

## الوصفة 8 — seek دقيق الإطار مع take_frame وrecycle

### الهدف

الانتقال إلى موضع والتحقق من زمن العرض الفعلي للصورة التي ظهرت، ثم إعادة مخزنها.

### استدعاءات API بالترتيب

1. `player.is_seekable()` (`src/player.rs:1010`)؛ البث الحي يُرفض (`src/player.rs:1112-1116`).
2. `player.seek(position)` (`src/player.rs:1110`) أو `player.seek_fraction(fraction)` (`src/player.rs:1136`).
3. انتظار صورة جديدة عبر `player.take_frame()` (`src/player.rs:1324`).
4. قراءة `frame.pts()` (`src/frame.rs:182`) و`frame.width()`/`height()` (`src/frame.rs:177`).
5. `player.recycle(frame)` (`src/player.rs:1333`).

### مخطط الكود

```rust,no_run
use std::time::{Duration, Instant};
use cranpose_video::{VideoError, VideoPlayer};

/// يُعيد زمن العرض للصورة التي ظهرت بعد الـ seek.
fn seek_and_show(
    player: &VideoPlayer,
    target: Duration,
    budget: Duration,
) -> Result<Option<Duration>, VideoError> {
    if !player.is_seekable() {                                   // src/player.rs:1010
        return Err(VideoError::UnsupportedOperation {
            operation: "seeking a live stream",
        });
    }
    player.seek(target)?;                                        // src/player.rs:1110
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if let Some(frame) = player.take_frame() {               // src/player.rs:1324
            let pts = frame.pts();                               // src/frame.rs:182
            upload(frame.width(), frame.height(), frame.rgba());
            present();
            player.recycle(frame);                               // src/player.rs:1333
            return Ok(Some(pts));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(None) // لم تظهر صورة خلال المدة التي حددها المتصل
}
```

### الفخاخ

- `seek` ليس دقيق الإطار بالمعنى التحريري: الموضع يُقصّ إلى `duration`، والصورة التالية هي أقرب صورة يستطيع المُفكِّك فكها من مفتاح سابق (`src/player.rs:1098-1105`). استخدم `frame.pts()` للتأكد.
- `[GAP]` لا يوجد `frame-step` في الواجهة العامة (`docs/FLEET-FULL-REVIEW-2026-11-21.md:31`). التنقل بإطار واحد يتطلب مُفكِّكاً مخصصاً أو `seek` بخطوة زمنية من التطبيق.
- رفض الـ seek من المنصة يظهر في `state()` لا في القيمة المُعادة؛ التعليق في `src/player.rs:1104-1105` يقول ذلك صراحة، والسلوك موثّق في `README.md:547`.
- `seek_fraction` يُعيد `UnsupportedOperation` عندما لا يُعلن الحاوي مدة (`src/player.rs:1127-1144`).
- المشغّل المتوقف ينشر صورة واحدة بعد كل `seek` (`src/player.rs:1318-1320`)؛ لا حاجة إلى `play` لمجرد رؤية الإطار.
- إذا فعّل التطبيق `set_native_nv12(true)` (`src/player.rs:1346`)، فالصورة تأتي بمستويات `luma` و`chroma` (`src/frame.rs:203,213`) ولا يمكن استدعاء `rgba()` عليها دون تحويلها (`src/frame.rs:223`).

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` `take_frame` يستدعي `pump` ثم يأخذ من الفتحة، و`recycle` يُعيد المخزن (`src/player.rs:1324-1335`).
- `[SOURCE-VERIFIED]` `VideoFrame::from_rgba8` يرفض حجماً لا يطابق `width * height * 4` (`src/frame.rs:130-140`, `tests/frame_matrix.rs:63-77`).
- `[SOURCE-VERIFIED]` المشغّل المتوقف ينشر صورة عند الفتح وبعد كل seek (`tests/playback.rs` و`src/player.rs:1318-1320`).
- `[HYPOTHESIS]` أن يطابق `pts` الذي يُعيده مُفكِّك المنصة الإطار المطلوب على جهاز حقيقي؛ دقة المفتاح خاصة بالمنصة.

## الوصفة 9 — تدهور رشيق عند فشل ترميز أو صيغة

### الهدف

إبقاء الصورة تعمل عند غياب الصوت أو فشل جزء من المصدر، وإظهار الفشل الحقيقي في الحالة والسجلات بدل ادعاء التشغيل السليم.

### استدعاءات API بالترتيب

1. `VideoPlayer::open(source)` (`src/player.rs:833`)؛ التعامل مع `VideoError` (`src/error.rs:8-34`).
2. `error.is_transient()` و`error.detail()` (`src/error.rs:38-52`) لقرار إعادة المحاولة.
3. عند غياب مُفكِّك المنصة: `VideoPlayer::with_decoder(...)` (`src/player.rs:901`).
4. في الحلقة: `state()` (`src/player.rs:982`) و`diagnostics()` (`src/player.rs:1312`).
5. `close()` (`src/player.rs:1378`).

### مخطط الكود

```rust,no_run
use cranpose_video::{VideoError, VideoPlayer, VideoState};

fn open_with_fallback(
    path: &str,
    fallback: impl FnOnce() -> Result<VideoPlayer, VideoError>,
) -> Result<VideoPlayer, VideoError> {
    match VideoPlayer::open(path) {
        Ok(player) => Ok(player),
        Err(VideoError::Unsupported { .. }) => fallback(),       // src/error.rs:11-14
        Err(error) if error.is_transient() => {                  // src/error.rs:50-52
            eprintln!("transient source failure: {:?}", error.detail());
            Err(error) // يقرر المتصل موعد إعادة المحاولة
        }
        Err(error) => Err(error),
    }
}

fn watch(player: &VideoPlayer) -> bool {
    let snapshot = player.diagnostics();                         // src/player.rs:1312
    match snapshot.state() {                                     // src/telemetry.rs:395
        VideoState::Failed(error) => {
            eprintln!("{:?} detail={:?}", error, error.detail());
            false
        }
        VideoState::Ended => false,
        _ => {
            if snapshot.errors() > 0 {
                eprintln!("recoverable errors so far: {}", snapshot.errors());
            }
            true
        }
    }
}
```

### الفخاخ

- مصدر بلا مسار صوت لا يُفشل `open`؛ غلّاف الصوت يتحول إلى no-op (`src/audio.rs:607-610`, `README.md:251`). لا تفسر غياب الصوت كفشل فيديو.
- فشل جهاز الصوت لا يُفشل الصورة: نتيجة تشغيل الصوت مُهمَلة في غلّاف المُفكِّك (`src/audio.rs:435-437`)، والصورة تواصل نشر الإطارات أثناء الاستعادة (`tests/audio_route_switch.rs:235-265`).
- `VideoState::Failed` ليس نهائياً بالضرورة: إطار ناجح بعد الاستعادة يُعيد المشغّل إلى `Playing` (`tests/audio_route_switch.rs:187-210`). لا تغلق المشغّل عند أول قراءة `Failed` دون مراعاة سجل الاستعادة.
- `VideoError::is_transient()` صحيح لحالة `Source` فقط (`src/error.rs:50-52`). لا تفترض أن `Decode` قابل لإعادة المحاولة.
- مصدر تالف يجب أن يستقر في `Failed` أو `Ended` أو `Paused`، لا أن يبقى "سليماً" بلا إطارات (`tests/malformed_source.rs:49-95`).
- `NoAudioTrack` و`NoVideoTrack` صنفان منفصلان (`src/error.rs:19-23`)؛ لا تستبدلهما بـ `Decode`.

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` الفشل يصل مرتين: من الاستدعاء ومن `VideoState::Failed` (`src/error.rs:1-6`, `tests/observability.rs:242-260`).
- `[SOURCE-VERIFIED]` حدث `Failed` وعدّاد `errors()` في اللقطة (`src/telemetry.rs:111-114`, `src/telemetry.rs:454-457`).
- `[SOURCE-VERIFIED]` مُفكِّك مخصص يعمل على هدف بلا مُفكِّك منصة (`src/player.rs:891-905`, `src/lib.rs:81-87`).
- `[SOURCE-VERIFIED]` فشل `hls.open` الدائم يُحوَّل إلى `VideoError::Decode` (`src/hls/mod.rs:1465-1474`).
- `[HYPOTHESIS]` أن يختار التطبيق بديلاً صحيحاً عند `Unsupported`؛ توفر مُفكِّك بديل يعتمد على حزمة التطبيق.

## الوصفة 10 — دورة الحياة بين الخلفية والمقدّمة

### الهدف

إيقاف العمل في الخلفية واستئنافه في المقدّمة، مع إغلاق آمن عند إتلاف الشاشة.

### استدعاءات API بالترتيب

1. عند الدخول للخلفية: `player.pause()` (`src/player.rs:1059`).
2. عند العودة للمقدّمة: `player.note_route_changed()` إن وُجد تغيّر جهاز (`src/player.rs:1244`)، ثم `player.play()` (`src/player.rs:1033`).
3. عند إتلاف الشاشة: `player.close()` (`src/player.rs:1378`) أو ترك `Drop` يفعل ذلك (`src/player.rs:1431-1435`).
4. شاشة التشخيص تتحقق من `is_closed()` (`src/player.rs:1020`) وتقرأ آخر حدث (`src/player.rs:1287`).

### مخطط الكود

```rust,no_run
use cranpose_video::{PlaybackEvent, VideoError, VideoPlayer};

struct Screen {
    player: VideoPlayer,
}

impl Screen {
    fn on_background(&mut self) {
        let _ = self.player.pause();                             // src/player.rs:1059
    }

    fn on_foreground(&self, route_may_have_changed: bool)
        -> Result<(), VideoError>
    {
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        if route_may_have_changed {
            self.player.note_route_changed();                    // src/player.rs:1244
        }
        #[cfg(target_arch = "wasm32")]
        let _ = route_may_have_changed;
        self.player.play()                                       // src/player.rs:1033
    }

    fn on_destroy(&mut self) {
        self.player.close();                                     // src/player.rs:1378
        debug_assert!(self.player.is_closed());                  // src/player.rs:1020
    }

    fn last_event(&self) -> Option<&PlaybackEvent> {
        self.player.events().last().map(|record| record.event()) // src/player.rs:1287
    }
}
```

### الفخاخ

- `play()` على مشغّل انتهى يُعيده إلى البداية إذا كان قابلاً للـ seek؛ البث الحي لا يُعاد تشغيله بعد الانتهاء (`src/player.rs:1039-1049`, `src/player.rs:1001-1004`).
- `close()` لا ينتظر مُفكِّكاً محجوباً (`src/player.rs:1372-1404`). لا تفترض أن خيط العمل انتهى عند عودة الاستدعاء.
- كل استدعاء بعد `close` يُعيد `VideoError::Closed` (`src/player.rs:1413-1419`)؛ `take_frame` يُعيد `None` (`src/player.rs:1325-1327`).
- `note_route_changed` غير موجود على Linux وwasm (`src/player.rs:1243`)؛ استخدم `cfg` كما في المخطط.
- استدعِ `set_muted(false)` بعد تفاعل المستخدم على الويب؛ المتصفح يبدأ العنصر كتماناً (`README.md:253`, `src/player.rs:1166-1174`).
- المكتبة لا تملك جلسة وسائط على مستوى النظام (`src/session.rs:7-9`)، ولا تستأنف وحدها عند عودة التطبيق؛ `[GAP]` دورة النشاط/المشهد من مسؤولية التطبيق (`docs/ANDROID-SUPPORT-FLEET-REVIEW.md:17`).
- جلسة الاستعادة تبقى تحاول ما دام المشغّل مفتوحاً، بلا حالة "استسلام" (`src/session.rs:38-42`)؛ لكنها لا توقظ خيطاً نائماً عند تغيّر المسار (`docs/MOBILE-RISK-REGISTER.md:142-150`).

### المصدر مقابل الجهاز

- `[SOURCE-VERIFIED]` `Drop` يستدعي `close` (`src/player.rs:1431-1435`)، و`close` يفصل الخيط بدل انتظاره (`src/player/threaded.rs:111-121`).
- `[SOURCE-VERIFIED]` `Closed` آخر حدث في سجل الاستعادة وسجل الأحداث (`src/session.rs:391-408`, `tests/observability.rs:278-288`).
- `[SOURCE-VERIFIED]` `play` يُعيد تشغيل مصدر انتهى ويُعيد `UnsupportedOperation` لمصدر حي انتهى (`src/player.rs:1039-1049`).
- `[HYPOTHESIS]` أن تحافظ دورة الحياة على الصوت عبر انتقالات Android الحقيقية؛ `[GAP]` لا يوجد اختبار جهاز هنا، و`docs/android-test-checklist.md:1-8` يذكر صراحة أن التحقق على الجهاز لم يُجرَ.

## خريطة سريعة من الحالة إلى الوصفة

| الحالة | الوصفة |
| --- | --- |
| خلاصة تمريرية بمشغّل واحد | 1 |
| شبكة مقاطع أو لوحة بعدة مشغّلات | 2 |
| فيديو فوق محتوى آخر | 3 |
| قناة حية HLS | 4 |
| صيغة لا يعرفها المنصة | 5 |
| تحميل المقطع التالي | 6 |
| سماعة Bluetooth أو فقدان تركيز الصوت | 7 |
| seek ومعايرة الإطار | 8 |
| ملف تالف أو جهاز صوت مشغول | 9 |
| خلفية/مقدّمة وإتلاف الشاشة | 10 |
