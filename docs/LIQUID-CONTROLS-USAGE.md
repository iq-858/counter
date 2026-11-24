# أدوات التحكم السائلة — الاستخدام والتحكم

## الحقيقة عن تطبيق counter
تطبيق `counter` (المشروع الرئيسي) يدفع المشغّل مباشرة (`VideoPlayer` داخل `Rc<Result<...>>`)
وليس `VideoPlayerHandle`. مكوّنات `#[composable]` في cranpose-ui تتطلب معاملات `PartialEq
+ 'static`، و`VideoPlayer` لا يمكن مقارنته (يملك خيوطاً وأقفالاً). لذلك **لا يمكن تركيب
أدوات الـ glass مباشرة على `&VideoPlayer`** — حاولناه ورفضه المترجم (E0277/E0521).

الوضع الحالي في `counter`: أزرار نصية تقليدية (`small_button`) — كما كانت.
هذا ليس فشلاً في التصميم، بل قيد تقني في نموذج composable.

## متى تعمل الأدوات السائلة؟
عندما يستخدم المشروع `VideoPlayerHandle` (من `rememberVideoPlayer`) — كما في示例
`cranpose-pwa/demo` أو أي مشروع يبني واجهته عبر `widget`.

## الواجهة المتاحة (في cranpose-video)
```rust,ignore
use cranpose_video::{
    VideoControlsLiquid, VideoControlsOptions, VideoLiquidIconButton,
    VideoLiquidAction, GlassStyle, ProControl,
};

// شريط كامل من handle:
VideoControlsLiquid(handle, Modifier::empty(), VideoControlsOptions::new()
    .style(GlassStyle::Floating)
    .elements(&[ProControl::PlayPause, ProControl::Mute, ProControl::Stop]));

// زر مفرد:
VideoLiquidIconButton(handle, Modifier::empty(), ICON, VideoLiquidAction::Toggle);
```

## الأزرار الستة (VideoControlsLiquid)
| الأيقونة | الإجراء |
|---|---|
| ▶ | `toggle()` |
| ‹‹ | `seek(position - 5s)` |
| ›› | `seek(position + 5s)` |
| ↺ | `seek(0) + play()` |
| ■ | `stop()` |
| 🔊 | `set_muted(!is_muted())` |

## التقنية
- أيقونات متجهة Material عبر `IconWith` (لا نصوص).
- `GlassIconButtonGroup` من `cranpose-liquid`: backdrop blur + tint + specular.
- أنيميشن `LiquidMotion::snappy()` عند الضغط.
- `content_description` لكل زر (accessibility).

## إن أردتَ الأدوات السائلة في counter فعلياً
يجب تحويل `ActiveReel` في `src/app_ui/reels.rs` لاستخدام `rememberVideoPlayer` + `VideoPlayerHandle`
بدل `Rc<Result<VideoPlayer,...>>`. هذا تغيير معماري في طريقة عمل الصفحة (كيفية الحصول على
الإطارات) — لم أفعله لأنه يكسر «مفكك واحد لكل صفحة» الحالي ويتطلب إعادة تصميم مضخة الإطارات.
مستعد لفعله في خطوة منفصلة إن وافقتَ على تغيير النموذج.
