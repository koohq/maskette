# Third-party files in this repository

Cargo and pnpm fetch Maskette's dependencies, so they are not kept here. The
files below are, and others made them:

| File | What it is | Made by | License |
| --- | --- | --- | --- |
| `frontend/public/models/anime-face-hrnetv2-int8.onnx` | The landmark model of [anime-face-detector](https://github.com/hysts/anime-face-detector): the HRNetV2 weights `mmpose_anime-face_hrnetv2.pth` from its v0.0.1 release, converted to ONNX and quantized to 8-bit integers, as found in Visual Studio Kigurumi | hysts | MIT ([below](#anime-face-detector)) |
| `frontend/public/models/blaze_face_short_range.tflite` | MediaPipe's [BlazeFace short-range face detection model](https://ai.google.dev/edge/mediapipe/solutions/vision/face_detector) (float16, version 1) | Google | Apache-2.0 ([LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt)) |
| `frontend/public/mediapipe-wasm/` | The WebAssembly module of the [@mediapipe/tasks-vision](https://www.npmjs.com/package/@mediapipe/tasks-vision) npm package, copied from it unchanged | The MediaPipe Authors | Apache-2.0 ([LICENSES/Apache-2.0.txt](LICENSES/Apache-2.0.txt)) |

`scripts/notices/nsis-COPYING.txt` is NSIS's own license file, kept so the
notices can name the parts of the installer.

## Everything in a build

A build of Maskette includes far more: the Rust crates compiled into the app,
the npm packages of the screen, the DejaVu fonts and the parts of the
installer. `pnpm notices` (which `pnpm bundle` runs) lists every one of them
with its license text. The app shows that list under Settings → About
Maskette → Third-party licenses, and each release attaches it as
`third-party-notices.txt`. `LICENSES/` holds the standard texts it uses for
packages that do not carry their own.

## anime-face-detector

```text
MIT License

Copyright (c) 2021 hysts

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
