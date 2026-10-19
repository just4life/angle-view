# Сторонние компоненты / Third-party components

## Модель BiRefNet

- Проект: https://github.com/ZhengPeng7/BiRefNet
- Авторы: Peng Zheng, Dehong Gao, Deng-Ping Fan, Li Liu, Jorma Laaksonen, Wanli Ouyang, Nicu Sebe.
  «Bilateral Reference for High-Resolution Dichotomous Image Segmentation», CAAI AIR 2024.
- Веса: `BiRefNet-general-bb_swin_v1_tiny-epoch_232`, ONNX-экспорт из https://github.com/danielgatis/rembg
  (release `v0.0.0`), переведены в safetensors скриптом `bg-removal/tools/convert_birefnet.py`.
- Лицензия: MIT.

## Крейты Rust

Все зависимости из `Cargo.lock` распространяются под разрешительными лицензиями
(MIT, Apache-2.0, BSD-2/3-Clause, Zlib, ISC, BSL-1.0, Unlicense, CC0-1.0, Unicode-3.0).
Исключение: `option-ext` (через `dirs`) под MPL-2.0, используется без изменений.
Полный список с лицензиями: `cargo metadata --format-version 1` или `cargo about generate`.

## Шрифты

egui встраивает шрифты Ubuntu Light (Ubuntu Font Licence 1.0), Hack (MIT), Noto Emoji и Emoji Icon Font (OFL-1.1).
