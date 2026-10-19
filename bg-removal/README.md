# Умное удаление фона на чистом Rust

Находит главный объект на фото (человек, животное, предмет) и аккуратно вырезает его, включая волосы и тонкие края. Результат: PNG с прозрачным фоном.

## Что внутри

- **Модель:** BiRefNet, вариант «lite» (backbone Swin‑T, веса `BiRefNet-general-bb_swin_v1_tiny-epoch_232`, лицензия MIT). Сравнивал с ISNet (u2net‑семейство): BiRefNet заметно лучше держит волосы, тонкие предметы и границы одежды.
- **Движок:** собственный CPU‑инференс на Rust (`src/swin.rs`, `src/conv.rs`, `src/birefnet.rs`). Готовые рантаймы не подошли: `tract` и ONNX Runtime через ONNX разворачивают deformable‑свёртки BiRefNet в очень медленные графы, а ORT ещё и нативная C++ библиотека. Результат сверен с эталонной PyTorch/ONNX моделью: расхождение маски не больше 0.00003.
- **Постобработка** (`src/matting.rs`):
  - оставляет только главный объект, мелкие оторванные куски фона убираются;
  - маска растягивается до исходного разрешения и уточняется guided‑фильтром по самому фото, только в полосе неуверенных пикселей у края;
  - цвет полупрозрачных пикселей (волосы, мех) пересчитывается (multi‑level foreground estimation), чтобы старый фон не просвечивал на новом.
- **Зависимости:** только Rust‑крейты (`gemm`, `rayon`, `safetensors`, `half`, `image`). Никакого Python, onnxruntime или C/C++ рантайма. Exe для Windows зависит только от системных DLL.

## Скорость

Около 8–9 с на фото 2–4 Мп на 4‑ядерной виртуальной машине, из них примерно 7 с работает сеть (она всегда считает в 1024×1024, независимо от размера фото). На обычном 6–8‑ядерном десктопе ожидается примерно 3–5 с. Модель загружается за 0.4 с и занимает 89 МБ (f16). Пиковое потребление памяти около 1.1 ГБ на фото 1816×2424.

## Windows: BgRemove-win64.zip

Скачивается со страницы [Releases](https://github.com/just4life/angle-view/releases/latest). Распакуйте в одну папку; файл модели `birefnet-lite.safetensors` должен лежать рядом с exe.

- **BgRemoveDemo.exe** — окно: «Открыть…» или перетащите фото, результат появится сам. Можно переключать фон (клетка, белый, чёрный, зелёный) и сохранить PNG.
- **bgremove.exe** — командная строка:

```
bgremove photo.jpg                     -> photo-nobg.png рядом с исходником
bgremove *.jpg -o out\                 -> пакетно в папку
bgremove photo.jpg --mask              -> плюс photo-mask.png (альфа-маска)
bgremove photo.jpg --bg FFFFFF         -> на белом фоне вместо прозрачности
bgremove photo.jpg --keep-all          -> не отбрасывать второстепенные объекты
bgremove photo.jpg --no-refine --no-decontaminate
```

## API для просмотрщика

```rust
use bg_removal::{BgRemover, Options};

let remover = BgRemover::from_file("birefnet-lite.safetensors")?; // один раз, держать в Arc
let img: image::RgbaImage = image::open("photo.jpg")?.to_rgba8();

let cutout: image::RgbaImage = remover.remove_background(&img, &Options::default()); // RGBA с прозрачным фоном
let mask: image::GrayImage   = remover.alpha_mask(&img, &Options::default());        // только маска 0..255
```

`Options`: `refine_edges`, `decontaminate_colors`, `main_subject_only` (все `true` по умолчанию) и `release_memory` (освобождать рабочие буферы после вызова; при пакетной обработке лучше `false`). `BgRemover` потокобезопасен (`Sync`), вызывать лучше из фонового потока, как в `demo/src/main.rs`.

Подключение: `bg_removal = { path = "bg-removal" }` (так его использует Angle View) или `bg_removal = { git = "https://github.com/just4life/angle-view" }`.

## Сборка

Из корня репозитория:

```
cargo build --release -p bg_removal        # bgremove
cargo build --release -p bgremove_demo     # BgRemoveDemo
```

Веса: `tools/convert_birefnet.py` переводит ONNX‑файл BiRefNet из [rembg](https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx) в `birefnet-lite.safetensors` (`python3 convert_birefnet.py model.onnx out.safetensors [f32]`). Нужен только для пересборки весов, программе Python не нужен.

## Ограничения

- Модель решает, что считать «главным объектом»: например, на фото со стулом часть стула может остаться вместе с человеком. Надписи и водяные знаки поверх фото иногда тоже сохраняются.
- Только CPU; GPU не используется.

## Лицензия

Код: MIT. Модель BiRefNet ([ZhengPeng7/BiRefNet](https://github.com/ZhengPeng7/BiRefNet)): MIT.
