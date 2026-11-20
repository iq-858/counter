# متطلبات بناء IPA من Windows عبر GitHub — `counter`

> مبني على دليل المطوّر الرسمي:
> https://github.com/samoylenkodmitry/Cranpose/blob/main/docs/guide.md#build-apple-apps-without-a-mac
>
> الحالة الحالية (2026-11-20): لا يوجد Mac محلي، SDK الخاص بـ Apple مرخّص فقط على أجهزة Apple.
> لذلك المسار الوحيد هو **GitHub-hosted macOS runner** عبر أداة `cranpose-build`.

## 1. لماذا لا يمكن بناء IPA محلياً على Windows؟
Apple SDK مرخّص فقط على أجهزة Apple. الدليل يقول صراحة: «An iOS build needs macOS and Xcode with the iOS SDK. From Windows or Linux, build on GitHub.» أي أن التجميع الحقيقي يجب أن يحدث على ماك — لكنه ماك سحابي مجاني من GitHub Actions (للمستودعات العامة).

## 2. الأدوات المطلوبة على هذا الجهاز
| الأداة | purpose | الحالة |
|---|---|---|
| `git` | رفع المشروع | موجود |
| GitHub CLI `gh` | تسجيل الدخول + إدارة المستودع + سحب الـ artifact | **مثبّت يدوياً** على `C:\Users\r\bin\gh.exe` (الإصدار 2.68.0). `winget` غير موجود على هذا الجهاز لذلك نُزّل من GitHub releases. |
| `cranpose-build` | ينشئ workflow ويطلق البناء على GitHub وينزّل الـ IPA | **يُثبَّت من Git** (غير موجود في crates.io): `cargo install --locked --git https://github.com/samoylenkodmitry/cranpose-build cranpose-build`. |
| Rust toolchain | موجود (rustc 1.99.1) | موجود |

Status at time of writing: `gh --version` → 2.68.0 OK؛ `gh auth status` → **not logged in** (the remaining blocker). `cranpose-build` installation was still running in the background.

## 3. ما أضفناه في المشروع لهذا الغرض
- `Cargo.toml`: `[lib]` بـ `crate-type = ["rlib","cdylib"]` + ميزات `ios`/`android` (تفعّل `cranpose/ios` و`cranpose/android`).
- `src/lib.rs`: نقطة دخول مشتركة + `cranpose::android_main!` (لا يوسّع على غير Android).
- `src/main.rs`: يستخدم `counter::run()`.
- `src/app_ui/reels.rs` + `live.rs`: ارتفاع الـ pager/الصورة مشتق من حجم النافذة (`LocalWindowState`) بدل ثوابت ثابتة — توافق مع الشاشات المختلفة.
- `CranposeBuild.toml`: `[app] name/identifier` + `[ios] plist`.
- `ios/Info.plist`: `CranposeSceneDelegate` + `CADisableMinimumFrameDurationOnPhone=true`.

## 4. ما المطلوب منك (لإعطائي الوصول إلى GitHub)
لا أستطيع استلام كلمة المرور داخل المحادثة بأمان. لديك طريقان:

**الطريقة A (موصى بها):** `gh` مثبّت بالفعل على `C:\Users\r\bin\gh.exe`. سجّل الدخول بنفسك:
```powershell
& 'C:\Users\r\bin\gh.exe' auth login
```
اختر `GitHub.com` → `HTTPS` → `Login with a web browser`، ثم الصق رمز الجهاز في المتصفح. منح الصلاحيات: `repo` و`workflow` و`read:org` (لازمة لرفع الـ workflow وسحب الـ artifact).
بعدها أخبرني «تم الدخول» وسأكمل كل شيء (`gh repo create`، `git push`، `cranpose-build init-ci`، `cranpose-build build --platform ios --on github`).

**الطريقة B:** أنشئ Personal Access Token بصلاحيات `repo`, `workflow`, `read:org` من https://github.com/settings/tokens واضبطه كمتغير بيئة:
```powershell
setx GH_TOKEN "ghp_xxxx"
```
ثم أغلق وأعد فتح الجلسة. لا تكتب التوكن في الدردشة أبداً.

## 5. خطوات البناء بعد الدخول (سأنفّذها أنا)
```powershell
git add -A
git commit -m "iOS and Android build support with responsive UI"
gh repo create counter --public --source=. --remote=origin --push
cranpose-build init-ci
git add .github/workflows/cranpose-build.yml CranposeBuild.toml
git commit -m "Build Apple packages on GitHub"
git push
cranpose-build build --platform ios --on github
```
الناتج: `dist/cranpose/ios/debug/counter.ipa` (غير موقّع).

## 6. التوقيع (أنت تتولىّه)
الدليل: الـ `.ipa` غير الموقّع يُثبَّت بأداة مثل xtool / AltStore / SideStore بحساب Apple مجاني (7 أيام، 3 تطبيقات لكل حساب). TestFlight/App Store يحتاج عضوية مدفوعة. أنت قلت ستوقّع يدوياً — لا حاجة لي بذلك.

## 7. قيود صريحة

- لا يمكنني الدخول إلى حسابك أو دفع أي شيء بدون أن تمنح الوصول عبر إحدى الطريقتين أعلاه.
- المستودع الحالي `D:\counter` **لا يحتوي على أي commit** ولا remote؛ سأحتاج إنشاء commit أولاً (هذا مطلوب للرفع).
- إذا كان المستودع `private`، فإن Actions يستهلك دقائق الحساب؛ `public` مجاني.
- لا يوجد Mac محلي؛ سأعتمد بالكامل على runner الخاص بـ GitHub.
- `cranpose-build init-ci` يكتب workflow تقبله GitHub فقط من الفرع الافتراضي — لذا سأدفعه إلى `master` أولاً ثم أشغّل البناء.
