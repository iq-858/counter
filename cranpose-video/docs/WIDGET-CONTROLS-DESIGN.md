# تصميم أدوات تحكم فيديو بنمط iOS: إضافة غير كاسرة لطبقة `widget`

> **تحديث 2026-11-22 [SOURCE-VERIFIED]:** نُفّذت الإضافات في `src/widget.rs` و`src/lib.rs`:
> `VideoPlayerHandle::stop()`، الأنواع `ProControl`/`GlassStyle`/`VideoControlsOptions`،
> وcomposables `VideoControlsPro` + `Video{Mute,Loop,Stop,Speed}Button` + `VideoVolumeSlider`.
> اختبارات التركيب أُضيفت في `tests/widget.rs`.
> **تحديث ثانٍ 2026-11-22 [SOURCE-VERIFIED] — تصميم أقوى:** أُضيفت مكوّنات
> `VideoControlsLiquid` و`VideoLiquidIconButton` + النوع `VideoLiquidAction`،
> وتعلية `cranpose-liquid 0.24.0` كـ dependency اختياري داخل ميزة `widget`.
> هذه المكوّنات تستخدم **أيقونات متجهة حقيقية** (مسارات Material Media مضمّنة
> في `widget.rs`) عبر `GlassIconButton`، و**تأثير glass حقيقي** (backdrop blur
> + tint + specular) عبر `GlassIconButtonGroup`، و**أنيميشن زنبركي حقيقي**
> (`LiquidMotion::snappy()` + `liquid_press_scale`) عند الضغط.
> `GlassIconButtonGroup` يوفّر `content_description` لكل زر ( accessibility ).
> المكوّنات القديمة (`VideoControlsPro` إلخ) ما زالت كما هي توافقاً.
> ** VoiceOver/Dynamic Type** تعتمد الآن على ما يوفّره liquid من دلالات،
> لكن السلوك الجهازي لا يزال `[HYPOTHESIS]` غير مُختبَر على جهاز.
> `cargo check` نظيف؛ اختبارات `cargo test` تصطدم بـ ICE في أداة الترجمة
> `rustc 1.99.1` على Windows مع `cranpose-ui-graphics` (مشكلة أداة، لا كود).
> **قيد تحقّق:** `cargo check` ناجح؛ اختبارات `cargo test` تصطدم بـ ICE في أداة
> الترجمة `rustc 1.99.1` على Windows مع `cranpose-ui-graphics` (مشكلة أداة، لا كود).
>
> **نوع المستند الأصلي:** تصميم واجهة عامة (API design) + مواصفات بصرية + خطة اختبار.
> **التاريخ:** 2026-11-20.
> **نطاق التعديل الأصلي:** **هذا الملف فقط.** لم تعدَّل أي شيفرة Rust، ولا `Cargo.toml`،
> ولا `README.md`، ولا أي ملف آخر. لم يُنفَّذ `commit` ولا أي عمل على Linux/Windows.
> **المدخل:** واجهة `widget` المُدقَّقة في `src/widget.rs` و`src/player.rs`. كل حقيقة «مؤكدة من المصدر» أدناه مُرفقة بـ `file:line` مقروء فعلياً.
> **قاعدة الأرقام:** لا توجد قياسات أداء/حرارة/ذاكرة في هذا المستند، ولا تُختلَق. أي ادّعاء كهذا يحتاج تشغيلاً على جهاز وسجلاً مسجّلاً (انظر `docs/phone-integration-review.md:273`, `docs/mobile-platform-audit.md:173`). الأرقام الوحيدة المستخدمة هي ثوابت مصدرية مُشار إليها أو قواعد HIG خارجية مُوسومة كذلك.

---

## 0. تصنيف الأحكام

| الوسم | المعنى |
|---|---|
| **【مؤكد من المصدر】** | حقيقة قابلة للتحقق من الشجرة الحالية مع `file:line`. تصف ما يفعله الكود **اليوم**. |
| **【فرضية】** | التصميم المقترح في هذا المستند؛ غير مطبّق. |
| **【قاعدة HIG】** | توجيه خارجي من Apple Human Interface Guidelines / Liquid Glass (iOS 26). ليس حكم شيفرة في هذه الشجرة. |
| **【غير مُختبَر على جهاز】** | أي سلوك زمن تشغيل/VoiceOver/لمس/حرارة؛ حتى لو كان منطقياً من التصميم. |

---

## 1. الوضع الراهن — ما هو مؤكد من المصدر

| البند | الحقيقة | المرجع |
|---|---|---|
| المقبض العام | `VideoPlayerHandle` يحمل `Rc<Result<VideoPlayer, VideoError>>` ويعرّف `PartialEq` عبر `Rc::ptr_eq` | `widget.rs:27-37` |
| دوال المقبض الموجودة | `play`, `pause`, `toggle`, `seek`, `seek_fraction`, `set_speed`, `set_looping`/`is_looping`, `set_muted`/`is_muted`, `set_volume`/`volume`, `position`, `duration`, `state`, `speed`, `info`, `decoded_size` | `widget.rs:39-150` |
| أزرار `VideoControls` اليوم | `play / pause`, `-5 s`, `+5 s`, `restart`, وسرعة — **لا** mute ولا volume ولا stop ولا looping | `widget.rs:191-272` (الأزرار عند `226-265`) |
| خطوة السعي | `SEEK_STEP = 5 s` | `widget.rs:14-15` |
| أبعاد شريط التقدّم الحالية | ارتفاع المسار `4.0`، قطر المقبض `12.0` ( logical pixels ) | `widget.rs:17-21` |
| مادة الرسم اليوم | ألوان صلبة عبر `Modifier.background(Color(..))` — لا يوجد blur/material في هذه الشجرة | `widget.rs:373-400` |
| `stop()` في المشغّل | موجود، يُعيد إلى الإطار الأول ويتوقف؛ يفشل بـ `UnsupportedOperation` للبث الحي | `player.rs:1036-1053` |
| `stop()` **غير معروض** على المقبض | `VideoPlayerHandle` لا يستدعي `VideoPlayer::stop()` | `widget.rs:39-150` |
| تحديث الحالة في التركيب | `LaunchedEffect` + `with_frame_nanos` يستطلع `position/state/speed` كل إطار | `widget.rs:192-203`, `widget.rs:287-292`, `widget.rs:322-332` |
| مضخة الإطار | `picture()` تقرأ `take_frame()` وتبني `ImageBitmap` داخل تأثير إطار واحد | `widget.rs:404-458` |
| التخزين keyed | `rememberVideoPlayer` يفتح مشغّلاً واحداً مفتاحه `source`; تغيير المصدر يُسقط القديم ويغلق خيوطه | `widget.rs:152-166` |
| سطح الأوامر | `play/pause/seek/set_speed/set_muted/set_volume/stop` تُرسل `Command` إلى الـ engine؛ `dispatch` مجرد `commands.send()` | `player.rs:64-81`, `player.rs:1041`, `player.rs:1061`, `threaded.rs:104-106` |

> **الحكم:** كل ما يفتقده الشريط الجاهز (mute/volume/stop/loop) **مدعوم أصلاً في الطبقة الأساسية**؛ المطلوب هو عرضه عبر واجهة composable جديدة فقط، دون لمس المشغّل أو المنصات. 【مؤكد من المصدر + فرضية】

---

## 2. المواصفات البصرية — iOS HIG / Liquid Glass

> هذه مواصفات **هدف تصميمي** 【فرضية】+【قاعدة HIG】. التنفيذ الفعلي في هذه الشجرة محدود بـ `Modifier.background(Color)` (`widget.rs:373-400`)؛ لا توجد مكوّنة material/blur في `cranpose-ui` داخل هذا المستند. لذلك تُوصَف المادة كـ **token بصري** (شفافية + vibrancy)، وتُطبَّق نسخة قابلة للتنفيذ بالألوان شبه الشفافة عبر الآلية الحالية. أي blur حقيقي يحتاج modifier جديداً في `cranpose-ui` — خارج نطاق هذه الإضافة. 【غير مُختبَر على جهاز】

### 2.1 البنية

```
┌───────────────────────────────────────────────┐
│  الصورة (picture)  —  ContentScale::Fit       │  widget.rs:446-454
│                                               │
│      ┌─────────────────────────────────┐      │
│      │  ▷   ‹‹   ››   ↺   ⏹   🔊 ━━🔊  ⟳  1x │  ← شريط glass عائم
│      │  ───────●──────────────────────  │      │  (ProgressSlider)
│      │  0:07 / 1:20 · playing · 1.00x     │      │  (status)
│      └─────────────────────────────────┘      │
└───────────────────────────────────────────────┘
```

- **نمطان للشريط** (`GlassStyle`): `Floating` — شريط عائم شفاف فوق الصورة (الافتراضي، الأقرب لـ Liquid Glass)؛ `Inline` — شريط مدمج تحت الصورة بنفس تخطيط `VideoControls` الحالي (`widget.rs:204-268`) كخيار توافق بصري.
- **المادة:** خلفية شبه شفافة داكنة مع حدّ رقيق (hairline) وتدرّج خفيف، فوق الصورة مباشرة؛ النص/Actions علوية بمقدار يكفي للقراءة. لا تُشفَّر الحالة باللون وحده (انظر §3).
- **الProgress:** يُعاد استخدام `VideoProgressSlider` كما هو (`widget.rs:278-299`)؛ لا تُغيَّر أبعاد المسار/المقبض (`widget.rs:17-21`) لأن تغييرها تغيير بصري لكل المستهلكين الحاليين. 【قاعدة HIG + مؤكد من المصدر】

### 2.2 الأيقونات والتسميات (SF Symbols — قاعدة HIG)

| العنصر | رمز مقترح | Label لـ VoiceOver |
|---|---|---|
| Play/Pause | `play.fill` / `pause.fill` | "Play" / "Pause" (تبديل حسب `state`) |
| Back | `gobackward.5` | "Back 5 seconds" (يستخدم `seek_step`) |
| Forward | `goforward.5` | "Forward 5 seconds" |
| Restart | `arrow.counterclockwise` | "Restart" |
| Stop | `stop.fill` | "Stop" — يُعيد للإطار الأول (`player.rs:1041`) |
| Mute | `speaker.fill` / `speaker.slash.fill` | "Mute" / "Unmute" (حسب `is_muted`) |
| Volume | `speaker.wave.2.fill` + slider | "Volume, adjustable" |
| Loop | `repeat` / `repeat.1` | "Loop off" / "Loop on" (حسب `is_looping`) |
| Speed | نص `{:.2}x` | "Playback speed, 1.00 times" |

- تُشتقّ التسميات من `state()`/`is_muted()`/`is_looping()`/`speed()` المقبضية (`widget.rs:125`, `97`, `84`, `133`) لا من حالة composable منفصلة، حتى لا ينفصل النص عن المشغّل. 【مؤكد من المصدر + فرضية】
- **Safe area / home indicator:** يُوضع الشريط داخل الحشوات الآمنة للنظام و above home indicator؛ لا شيء تحت notch. 【قاعدة HIG】

---

## 3. قواعد التفاعل والوصولية

> كل ما يلي 【قاعدة HIG】+【فرضية】ما لم يُوسم بـ «مؤكد من المصدر».

### 3.1 أهداف اللمس (44pt)

- كل زر **<= 44×44 point** مساحة قابلة للضغط، حتى لو كان الرمز أصغر بصرياً. هذا حدّ HIG أدنى وليس رقماً مُخترعاً.
- المسافة الدنيا بين الأهداف كافية لمنع اللمس الخاطئ؛ الشريط الحالي يستخدم `SpacedBy(6.0)` و`padding(2.0/8.0)` (`widget.rs:206`, `218`, `481`, `487`) — تُحترم كخط أساس، وتُضبط في composable الجديد دون تعديل القديم.

### 3.2 VoiceOver

- كل زر يحمل `accessibility label` و`traits = button`؛ شريط الصوت يحمل `adjustable` مع `/value` تُحدَّث من `volume()` (`widget.rs:110`).
- بعد تبديل mute/loop/speed، يُعلَن تغيير الحالة (مثلاً "Muted", "Loop on") بدل الاعتماد على تغيّر اللون فقط.
- ترتيب التركيز يتبع الترتيب البصري من اليسار إلى اليمين؛ عناصر الموضع/المدة/الحالة في سطر واحد تُقرأ كنص واحد (كما يفعل `status()` في `widget.rs:495-512`).
- لا يُشفَّر أي وسم خطورة/حالة باللون وحده — كل حالة تقترن بأيقونة أو نص (قاعدة مطابقة لما التزم به التقرير السابق في `phone-integration-review.md:47`).

### 3.3 Dynamic Type

- نصوص الحالة والسرعة تتبع حجم خط النظام عبر `TextStyle`؛ لا تُثبَّت أحجام نقطة داخل composable الجديد.
- عند تكبير النص، يلتفّ الشريط/يتمدد ولا يُقتطع؛ لا يعتمد التخطيط على عرض ثابت للنص.
- **`Reduce Transparency`:** يُستبدل الخلفية الشفافة بخلفية معتمة في وضع glass. **«Reduce Motion»:** لا حركة انتقال عند ظهور الشريط. 【غير مُختبَر على جهاز】

### 3.4 منطق التفاعل

- ضغطة واحدة = أمر واحد على المقبض؛ لا حلقات متزامنة تستدعي `seek`/`set_speed` من composable.
- السرعة تَدور على `speeds` (الافتراضي `[0.5, 1.0, 2.0]` مطابق لدورة `widget.rs:255-261`) وتكتب عبر `set_speed()`؛ المدى المدعوم `0.0625x..8x` (`player.rs:1103`) يُ clamp في المشغّل.
- `Stop` يستدعي المقبض `stop()` الجديد؛ على بث حي يُرجع `UnsupportedOperation` (`player.rs:1043-1047`) فيُخفى الزر أو يُعطَّل عندما `duration() == None` — نفس منطق تعطيل الـ slider في `widget.rs:284`.
- `Mute` و`Volume` مستقلان: إلغاء الكتم لا يرفع الصوت من الصفر؛ هذا مذكور صراحة في واجهة المقبض (`widget.rs:101-112`).

---

## 4. إبقاء كل عمل الجهاز/الصوت خارج خيط التركيب

> 【مؤكد من المصدر للآلية الحالية + فرضية للالتزام الجديد】

1. **الفصل قائم أصلاً:** أوامر التحكم تُرسل إلى الـ engine عبر `Command` (`player.rs:64-81`)؛ `dispatch()` هو `commands.send()` فقط — غير حاجز لخيط التركيب (`threaded.rs:104-106`). composables الجديدة تستدعي **طرق المقبض فقط** (`set_muted`, `set_volume`, `set_looping`, `set_speed`, `seek`, `stop`) ولا تلمس AVFoundation / cpal / MediaCodec مباشرة.
2. **الصوت يتبع نفس المسار:** `set_muted`/`set_volume` يترجمان إلى `Command::SetMuted`/`Command::SetVolume` (`player.rs:75-78`, `1136`, `1160`)؛ عمل الجهاز الصوتي يبقى على خيط الـ engine/الصوت الأصلي، لا في التركيب.
3. **القراءات shared-state:** `position/state/speed/volume/is_muted/is_looping/duration` تُقرأ من `Shared` في المشغّل (`player.rs:939-972`, `1147`, `1172`, `1296`)؛ تُستطلع من `LaunchedEffect` عبر `with_frame_nanos` كما في `widget.rs:198-203`. لا I/O متزامن داخل composable.
4. **تحذير السعي:** المُدخل يحذّر من حجب الخيط عند `seek` (`phone-integration-review.md:58`)؛ يمثل هذا **الخيط الأصلي للمنصة** (مثلاً إعادة بناء `AVAssetReader` عند `open_at`) لا خيط التركيب — المقبض يرسل أمراً فقط (`player.rs:1061`, `threaded.rs:104-106`). لذلك: أمر seek واحد لكل إيماءة، بلا تكرار في حلقة الإطار.
5. **مضخة الإطار واحدة:** فك الإطار وتحويله إلى `ImageBitmap` يبقى حصراً داخل `picture()` (`widget.rs:404-458`)؛ الأدوات الجديدة **لا** تفك ولا تحوّل ولا تبني bitmap.
6. **توحيد الاستطلاع:** بدل أن تفتح كل أداة `LaunchedEffect` حلقة إطار خاصة بها للـ mute/volume/loop/status، تُستخدم حلقة tick واحدة مشتركة (نمط `tick` في `widget.rs:196-203`) تُمرَّر للمكوّنات، لتفادي تضاعف حلقات الاستطلاع. 【فرضية】

> **قيد صريح:** لا تُستدعى دوال المنصة Apple/Android مباشرة من طبقة `widget`؛ أي إضافة من هذا النوع تُرفض لأنها تكسر الحاجز الموصوف في `phone-integration-review.md:277-300`.

---

## 5. تكامل «مفكّك واحد في كل لحظة»

> 【مؤكد من المصدر للآلية + فرضية للقاعدة】

- **المبدأ:** الأدوات الجديدة تأخذ `VideoPlayerHandle` **كمعلمة** (تماماً كـ `VideoControls(handle, modifier)` في `widget.rs:191`) ولا تستدعي `rememberVideoPlayer` داخلياً أبداً. بهذا لا تُنشئ أي أداة مفكّكاً إضافياً.
- **المفتاح:** مشغّل واحد لكل مصدر عبر `rememberKeyed` (`widget.rs:163-164`)؛ تغيير المصدر يُسقط المشغّل القديم ويغلق خيوطه (`widget.rs:159-162`). هذا هو الأساس الحالي لـ«واحد في كل لحظة».
- **الحكم المنهجي:** «مفكّك واحد في كل لحظة» هو **ثابت على مستوى التركيب** (composition-level invariant) وليس عدّاداً زمنياً مقاساً — كما ورد صراحة في `docs/hardening-audit-plan-2026-11-19.md:55,158`. لا يُدّعى قياس runtime.
- **تسليم النشاط (Reels):** على مستوى الشاشة (خارج هذه الإضافة) يبقى مقبض واحد نشط. عند الانتقال لمقطع جديد: يُوقَف/يُستبدل المقبض القديم أولاً، ثم يسمح `rememberKeyed` بإسقاطه؛ `stop()` الجديد (`player.rs:1041`) مفيد كحالة «إعادة للبداية وتحرير عمل الفك» قبل الإسقاط. لا تُفتح حلقة `with_frame_nanos` ثانية لنفس المقبض.
- **البث الحي:** `stop`/`seek` يفشلان للبث غير القابل للسعي (`player.rs:1043-1047`, `1069-1072`)؛ تُعطَّل الأزرار المرتبطة عندما `duration() == None` (نمط `widget.rs:284`).
- **لا تعديل على:** `player.rs`, `player/threaded.rs`, `platform/*`, `Cargo.toml`. الإضافة كلها في `widget.rs` + `lib.rs`.

---

## 6. مخطط الـ API العام — إضافات فقط

> القاعدة: **لا تغيير لأي توقيع قائم.** كل ما يلي أنواع/دوال/طرق جديدة. إضافة طريقة inherent إلى `VideoPlayerHandle` وإضافة أسماء إلى `pub use` كلاهما غير كاسر. 【فرضية】

### 6.1 طريقة جديدة على المقبض (تعريض `stop`)

```rust
impl VideoPlayerHandle {
    /// Returns to the first frame and stops there.
    /// Surfaces `VideoPlayer::stop()` (player.rs:1041), currently unreachable
    /// from the public handle. Fails with `UnsupportedOperation` for live
    /// sources (player.rs:1043-1047).
    pub fn stop(&self) -> Result<(), VideoError> {
        self.player()?.stop()
    }
}
```

### 6.2 أنواع جديدة

```rust
/// عنصر واحد في شريط التحكم الموسّع.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProControl {
    PlayPause,
    Back,
    Forward,
    Restart,
    Stop,
    Mute,
    Volume,
    Loop,
    Speed,
}

/// موضع مادة الـ glass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassStyle {
    Floating, // شريط عائم فوق الصورة (الافتراضي)
    Inline,   // شريط مدمج تحت الصورة، مطابق لتخطيط widget.rs:204-268
}

/// تهيئة ثابتة لـ `VideoControlsPro` — لا تُعدَّل بعد الإنشاء.
#[derive(Clone)]
pub struct VideoControlsOptions {
    pub elements: &'static [ProControl],
    pub seek_step: Duration,
    pub speeds: &'static [f32],
    pub style: GlassStyle,
    pub show_progress: bool,
    pub show_status: bool,
}

impl Default for VideoControlsOptions {
    fn default() -> Self {
        Self {
            elements: &[
                ProControl::PlayPause, ProControl::Back, ProControl::Forward,
                ProControl::Restart, ProControl::Stop, ProControl::Mute,
                ProControl::Volume, ProControl::Loop, ProControl::Speed,
            ],
            seek_step: SEEK_STEP,       // widget.rs:15  (5 s)
            speeds: &[0.5, 1.0, 2.0],   // widget.rs:255-261 (دورة السرعة الحالية)
            style: GlassStyle::Floating,
            show_progress: true,
            show_status: true,
        }
    }
}

impl VideoControlsOptions {
    pub fn new() -> Self { Self::default() }
    pub fn elements(mut self, elements: &'static [ProControl]) -> Self { self.elements = elements; self }
    pub fn seek_step(mut self, step: Duration) -> Self { self.seek_step = step; self }
    pub fn speeds(mut self, speeds: &'static [f32]) -> Self { self.speeds = speeds; self }
    pub fn style(mut self, style: GlassStyle) -> Self { self.style = style; self }
    pub fn show_progress(mut self, show: bool) -> Self { self.show_progress = show; self }
    pub fn show_status(mut self, show: bool) -> Self { self.show_status = show; self }
}
```

### 6.3 composables جديدة

```rust
/// شريط تحكم موسّع بنمط iOS فوق مقبض يملكه المستدعي.
/// لا يفتح مشغّلاً جديداً — يستهلك المقبض المُمرَّر فقط.
#[composable]
pub fn VideoControlsPro(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    options: VideoControlsOptions,
) { /* ... */ }

/// أزرار/شرائح مستقلة، كلها تأخذ المقبض كمعلمة (لا rememberVideoPlayer داخلها).
#[composable]
pub fn VideoMuteButton(handle: VideoPlayerHandle, modifier: Modifier) { /* ... */ }
#[composable]
pub fn VideoVolumeSlider(handle: VideoPlayerHandle, modifier: Modifier, spec: SliderSpec) { /* ... */ }
#[composable]
pub fn VideoLoopButton(handle: VideoPlayerHandle, modifier: Modifier) { /* ... */ }
#[composable]
pub fn VideoStopButton(handle: VideoPlayerHandle, modifier: Modifier) { /* ... */ }
#[composable]
pub fn VideoSpeedButton(
    handle: VideoPlayerHandle,
    modifier: Modifier,
    speeds: &'static [f32],
) { /* ... */ }
```

> `SliderSpec` مستورد أصلاً في `widget.rs:9`؛ لا استيراد جديد مطلوب. `#[composable]` هو النمط الحالي (`widget.rs:172`, `180`, `190`).

### 6.4 إعادة التصدير (إضافة أسماء فقط) — `src/lib.rs:57-61`

```rust
#[cfg(feature = "widget")]
pub use widget::{
    Video, VideoControls, VideoPlayerHandle, VideoProgressSlider, VideoProgressSliderWith,
    VideoWithControls, progress_fraction_for, rememberVideoPlayer,
    // ↓ إضافة فقط:
    GlassStyle, ProControl, VideoControlsOptions, VideoControlsPro,
    VideoLoopButton, VideoMuteButton, VideoSpeedButton, VideoStopButton, VideoVolumeSlider,
};
```

### 6.5 ما **لا** يتغيّر (صريحاً)

- توقيعات `VideoControls`, `Video`, `VideoWithControls`, `VideoProgressSlider`, `VideoProgressSliderWith`, `progress_fraction_for`, `rememberVideoPlayer` (`widget.rs:156-370`).
- الثوابت `SEEK_STEP`, `PROGRESS_TRACK_HEIGHT`, `PROGRESS_THUMB` (`widget.rs:14-21`) — لا تُعدَّل قيمها.
- `VideoState` (`state.rs:9-25`) و`Command` (`player.rs:64-81`) وسلوك المشغّل/المنصات.

---

## 7. موضع الملفات

| ما يُضاف | الملف | الموقع |
|---|---|---|
| ثوابت/builder/anواع جديدة + `stop()` + composables جديدة | `src/widget.rs` | قسم «additive» في نهاية الملف (بعد `clock()` عند `widget.rs:514-519`)؛ `stop()` يُوضع في `impl` منفصل جديد أو داخل `widget.rs:39-150` دون تعديل تواقع القائمة |
| إعادة تصدير الأسماء الجديدة | `src/lib.rs` | توسيع قائمة `pub use widget::{...}` عند `57-61` |
| اختبارات التركيب | `tests/widget.rs`, `tests/widget_state.rs` + ملف جديد `tests/widget_pro_controls.rs` | نمط `compose_and_measure` عند `tests/widget.rs:15-22` |
| **لا يُلمس** | `player.rs`, `player/*.rs`, `platform/*`, `Cargo.toml`, `README.md` | — |

> كل اختبارات widget محكومة بـ `#![cfg(feature = "widget")]` (`tests/widget.rs:2`, `tests/widget_state.rs:7`)؛ الميزة افتراضية (`Cargo.toml:16`, `19`).

---

## 8. خطة الاختبار

### 8.1 اختبارات آلية (بدون جهاز)

| # | الفحص | المكان | المعيار |
|---|---|---|---|
| T1 | `VideoControlsPro` بتهيئة افتراضية يتركّب بلا panic | `tests/widget_pro_controls.rs` | نمط `ready_made_controls_compose_from_an_existing_handle` (`tests/widget.rs:24-30`) |
| T2 | كل composable مستقل (mute/volume/loop/stop/speed) يتركّب | نفسه | `compose_and_measure` (`tests/widget.rs:15-22`) |
| T3 | `GlassStyle::Inline` يتركّب بنفس مسار `VideoControls` القديم | نفسه | بلا panic + لا تغيير على القديم |
| T4 | `VideoControlsOptions::default()` يعطي `seek_step == SEEK_STEP` و`speeds == [0.5,1.0,2.0]` | اختبار وحدة | مطابق `widget.rs:15`, `255-261` |
| T5 | `progress_fraction_for` لم يتغيّر | `tests/widget.rs:53-81` | الحالات الحالية تمرّ كما هي |
| T6 | مضخة الإطار مع شريط موسّع تظل حلقة واحدة | `tests/widget_state.rs:23-35` | نمط `the_controls_compose_with_a_status_tick` |
| T7 | المقبض `stop()` يترجم إلى `VideoPlayer::stop()` | اختبار شيفرة/مراجعة | مرجع `player.rs:1041`؛ بث حي يُرجع `UnsupportedOperation` (`1043-1047`) |
| T8 | سرعة لا تتجاوز المدى عند الدوران | اختبار وحدة | الـ clamp في `player.rs:1103`؛ القيم `[0.5,1.0,2.0]` ضمن المدى |

### 8.2 فحوص يدوية/على جهاز 【غير مُختبَر على جهاز】

- VoiceOver: ترتيب التركيز، تسميات الحالة بعد التبديل، وسم الـ `adjustable` لشريط الصوت وقراءة قيمته.
- Dynamic Type: تكبير الخط حتى أقصى إعداد؛ لا اقتطاع، الالتفاف سليم.
- 44pt: قياس مساحة اللمس الفعلية على جهاز، دون الاعتماد على الحجم البصري للرمز.
- Reduce Transparency / Reduce Motion: بديل معتم، بلا حركة ظهور.
- Reels: مقطعان متتاليان — التأكد من بقاء مفكّك واحد نشط وعدم ظهور إطار قديم بعد التبديل (القاعدة في §5؛ احذر من الادّعاء بقياس runtime — `hardening-audit-plan-2026-11-19.md:158`).

### 8.3 بند قياس محجوز

> **[MEASURED — TO BE FILLED AFTER DEVICE RUN]:** لا تُدخل أرقام إطار/حرارة/زمن لمس قبل تشغيل فعلي بسجل command وبيئة ومخرجات. تُملأ من ذلك السجل فقط، ولا تُختلَق الأرقام (`phone-integration-review.md:273`).

---

## 9. ملخص القرار

الإضافة قابلة للتحقيق **بالكامل داخل `widget.rs`** كأنواع وcomposables جديدة + طريقة `stop()` واحدة على المقبض + سطر re-export في `lib.rs`. لا تتغير أي توقيعات، لا تُنشأ مفكّكات إضافية، ولا ينتقل أي عمل جهاز/صوت إلى خيط التركيب. الجوانب البصرية/الوصولية هي أهداف HIG مُوسومة كفرضيات حتى تُختبَر على جهاز. 【فرضية】
