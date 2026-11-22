# خارطة طريق تطوير cranpose-video — التطورات المستقبلية

> تاريخ: 2026-11-22. الوسوم: `[SOURCE-VERIFIED]` مقروء من المصدر الحالي،
> `[HYPOTHESIS]` يحتاج جهازاً/قياساً. الأرقام هي أرقام أسطر لا قياسات أداء.
> هذا ملف تخطيط **لا يغيّر الكود**؛ كل بند يشير إلى الدليل من المراجعات في `docs/`.

## 1. السطح العام (Public API) — فجوات مؤكدة

| الأولوية | البند | الدليل | الهدف |
|---|---|---|---|
| P1 | `VideoPlayerHandle` لا يعرض `set_audio_group`/`take_frame`/`recycle`/`close`/`with_decoder` | `widget.rs:39-149` مقابل `player.rs:901,1230,1324,1378` | تطبيق يبني واجهته الخاصة لا يستطيع التحكم بالصوت أو الإطارات من المقبض. أضف تفويضات (delegates) دون تغيير التوقيعات. |
| P1 | `seek_fraction`Panics على NaN بينما `set_speed`/`set_volume` تحميان | `player.rs:626-634` مقابل `:646-650,:496-500` | تمرير قيمة من شريط/نص قد يُسقط التطبيق. تحمي بـ `is_finite()`. |
| P2 | `VideoFrame::from_rgba8` يقبل buffer قصير عبر فيض `usize` | `frame.rs:22-30` | استخدم `checked_mul`. |
| P2 | `state()` يستنسخ سلسلة الخطأ في كل استطلاع | `player.rs:164-166`, `state.rs:24` | أضف `is_failed()`/`failure_kind()` دون تخصيص. |
| P2 | `VideoSource` يفتقر `&String`/`&PathBuf`/`&[u8]`/`Arc<str>` | `source.rs:94-136` | أضف `From` impls إضافية (إضافة فقط). |
| P2 | `speed_range()` عام مفقود | `player.rs:45-49` | ثوابت `MIN_SPEED`/`MAX_SPEED` خاصة؛ واجهات المستخدم تُعيد كتابتها. |
| P3 | أسماء composables تخالف `C-CASE` | `widget.rs:105-129` | أضف aliases snake_case دون حذف القديمة. |

## 2. ملاءمة المشاريع (Project fit) — `[SOURCE-VERIFIED]`

- **صوت فقط (audio-only)**: `VideoSource` له `Path`/`Url`/`Bytes` فقط (`source.rs:11-19`) — لا نوع صوتي، لا واجهة صوت عامة (كل طرق `Audio` `pub(crate)`). أضف مسار audio-only أو `VideoDecoder` يبث صوتاً بلا إطارات.
- **مصادر متسابقة (first-frame-wins)**: لا primitive لذلك. أضف `race(sources)` فوق عدة `VideoPlayer`.
- **Frame-step / `seek_to_live`**: `seek`/`take_frame` موجودان (`player.rs:1110,1324`) لكن لا خطوة إطار ولا قفز إلى الحافة الحية.
- **PiP نظامي**: لا `AVPictureInPictureController` في المكتبة — مسؤولية التطبيق (`docs/CROSS-PLATFORM-PARITY.md`). وثّق ذلك بوضوح في README.

## 3. الأداء (Performance) — `[SOURCE-VERIFIED]` ما لم يُعلَم `[HYPOTHESIS]`

1. **`open` متزامن على خيط التركيب** — `widget.rs:164` → `VideoPlayer::open`. أضف `open_async` يُرجع handle `Loading` فوراً (حالة `Loading` موجودة في `state.rs:13-16`). `[HYPOTHESIS]` قياس الزمن على كل منصة.
2. **`take_frame` لا يُعيد recycle** — `widget.rs:429-433` يستدعي `into_rgba()` دون `recycle`، فيُستنزف تجمع `Slot::spare` (`player.rs:317-318`). أضف `from_video_rgba8` يتخطى التجزئة/فحص alpha (`image.rs:395-404`) وأعد الإطار بعد الرسم.
3. **تحويل YUV scalar على الهاتف** — `android.rs:1479-1497` حلقة لكل بكسل بلا NEON. `[HYPOTHESIS]` المهيمن عند 4K. أضف مسار NEON مع oracle اختباري (`pixels.rs`).
4. **`BufferInfo` لكل إطار على Android** — `android.rs:312-318` تخصيص JNI لكل إطار. احتفظ بمرجع global وأعد استخدامه.
5. **انتظار 4/10/100 ms بالـ sleep** — `threaded.rs:23,269-301`, `engine.rs:47`. استبدل بـ Condvar/إشارة عند النشر.
6. **تقييد 60fps غير موجود** — `player.rs:53,332-345`. قرار تطبيق؛ وثّق سلوك 30fps على شاشة 120Hz `[HYPOTHESIS]`.
7. **N مشغّل = N خيوط بلا منسّق** — `player.rs:816-821`. أضف executor مقيد وإيقاف المشغّلات خارج الشاشة. `[HYPOTHESIS]` قياس الحرارة عند N=8.

## 4. أخطاء الشبكة (Resilience) — `[SOURCE-VERIFIED]`

- **التصنيف يُمحى بين الطبقات** — `hls/mod.rs:525-527,1566,1733` تُسقط `Failure.transient`، و`note_failure` (`1421-1426`) يعتمد على مطابقة نص `answered {code}`. بدّله بحقل HTTP status منظّم في خطأ الجلب.
- **حلقة استعادة لا تنتهي** — `session.rs:297-298` صراحة «لا حالة توقف». أضف حالة توقف/تقييد بعد سقف محاولات مع إبقاء إمكانية إعادة المحاولة اليدوية.
- **فشل صوت HLS صامت** — `hls/mod.rs:457-463,902-924` يبتلع أخطاء rendition الصوت. أضف `diag WARN` و`SessionEvent::Failed{stage:"hls.audio"}`.
- **`Retry-After`** يُحلل كثوانٍ فقط ويُقصّ إلى 5s (`hls/mod.rs:2066-2072`) — أضف صيغة HTTP-date وسقفاً أكبر موجهاً من الخادم.
- **إعادة جلب كاملة بعد فشل جزئي** — `fetch_many` (`1713-1726`) يُسقط النجاحات. احفظها في cache قبل إعادة الخطأ.

## 5. الأمن/الخصوصية (Security) — `[SOURCE-VERIFIED]`

- **CDN script بلا SRI** — `platform/web.rs:39,175-203` يحقن `hls.js@1` من jsDelivr. اطلب من التطبيق توفير البناء أو ثبّت إصداراً مع `integrity`.
- **تسريب توكن في `VideoError` المُعاد** — `apple.rs:112-118`, `apple_avplayer.rs:118-119` يدمجان الرابط الخام. أعدّله عند الإنشاء لا عند التخزين فقط.
- **SSRF عبر playlist** — `hls/playlist.rs:1002-1003` يقبل عناوين مطلقة، `hls/mod.rs:1902` يجلبها بلا تحقق. أضف allowlist للorigin ورفض loopback/link-local.
- **تحويلات غير محقّقة** — `hls/mod.rs:2024` `.redirects(5)` يسمح HTTPS→HTTP. أضف سياسة تتحقق من كل قفزة.
- **ملفات مؤقتة بصلاحيات عالمية** — `temp_file.rs:20-38` أسماء متوقعة + `fs::write` يتبع symlinks. استخدم `O_EXCL|O_NOFOLLOW` و0600.
- **sink يُستدعى تحت mutex** — `diagnostics.rs:42-52` يستدعي callback أثناء القفل. انسخ المؤشر وأفلِت القفل قبل النداء.

## 6. تكافؤ المنصات (Parity) — انظر `CROSS-PLATFORM-PARITY.md`

- `set_audio_group` no-op على Windows وكل مسارات HLS — مطلوب توحيد المالكين الأربعة (`audio.rs`, `hls/mod.rs`, `apple_avplayer.rs`, `windows/mf_audio.rs`) خلف واجهة واحدة.
- `carriers_audio` يختلف بالمنصة (`windows.rs:1055`, `apple_avplayer.rs:220`، الباقي افتراضي `false`) — توحيده يمنع صوتاً مزدوجاً أو مفقوداً.
- `attach_session`/`recover` مُخفيان بـ cfg على Linux/wasm (`decoder.rs:221,231`) — قسّم الـ trait إلى core + capability traits لجعل التوافق صريحاً.
- **ما يجب على التطبيق فعله** (لا المكتبة): Android audio focus/FGS/PiP/wake lock، iOS AVAudioSession category/activation/interruption/PiP/HDR. تتطلب جسر Java/Kotlin أو Info.plist.

## 7. جودة الاختبارات — انظر QA inventory

- **لا اختبار لـ `VideoPlayer::open_hls`** (`player.rs:854`) رغم أهميته — أضف خادم HTTP محلي (`tests/resilience.rs`现有 نموذج).
- **لا اختبار iOS AVPlayer/JNI** — تتطلب محاكي/جهاز؛ أضف `#[cfg(test)]` modules gated بالمنصة.
- **تغطية `HLS errors`** وحدة فقط — أضف اختبارات end-to-end لـ 403/DNS/timeout/discontinuity.
- أضف: `close` متكرر/آمن، عدد خيوطbounded، `a_stale_group_owner_cannot_write_to_output`.

## 8. الترتيب المقترح (Dependency order)

1. توحيد مالكي الصوت + إصلاح `release()`/promotion (تم 2026-11-21) — بوابة لكل شيء.
2. `open_async` + recycle صحيح (أكبر أثر على سلاسة Live).
3. أخطاء شبكة منظّمة + circuit breaker.
4. سطح Handle + audio-only + source racing.
5. أمن (redaction/SSRF/SRI) قبل أي إصدار عام.
6. SIMD/NEON وقياسات جهازية بعد كل ما سبق.

> **الحكم الحالي `[SOURCE-VERIFIED]`**: البنية سليمة (SOUND) لكن **ليست جاهزة
> للإنتاج** بلا إثبات جهازي وتغطية parity. انظر `FLEET-FULL-REVIEW-2026-11-21.md`
> و `REVIEW-RECHECK-2026-11-21.md`.
