## 鸣谢 Acknowledgements

壹鸣的抠图能力建立在以下开源模型之上，向它们的作者与维护团队致以诚挚感谢。各模型权重版权归原项目所有，请遵循其对应许可证使用。

### MODNet · 人像实时抠图

- 作者：Zhanghan Ke（柯彰翰）等，香港城市大学
- 项目：[ZHKKKe/MODNet](https://github.com/ZHKKKe/MODNet)
- 论文：*MODNet: Real-Time Trimap-Free Portrait Matting via Objective Decomposition*（AAAI 2022）
- 许可证：Apache-2.0
- 用途：人像发丝级 alpha 抠图，证件照换底的主力模型（`modnet.onnx`）

### BiRefNet · 高分辨率二分图像分割

- 作者：Peng Zheng（@ZhengPeng7）等
- 项目：[ZhengPeng7/BiRefNet](https://github.com/ZhengPeng7/BiRefNet)
- 论文：*Bilateral Reference for High-Resolution Dichotomous Image Segmentation*（CAAI AIR 2024）
- 许可证：MIT
- 用途：通用生活场景的高精度抠图（`birefnet_1024.onnx`）

### BEN2 · 背景移除网络

- 作者：Prama LLC（@PramaLLC）
- 项目：[PramaLLC/BEN2](https://github.com/PramaLLC/BEN2) · [Hugging Face](https://huggingface.co/PramaLLC/BEN2)
- 许可证：MIT（Base 开源版）
- 用途：电商产品图与通用场景背景移除（`BEN2_Base.onnx`）

### InSPyReNet · 高分辨率显著目标检测

- 作者：Taehun Kim（@plemeri）等，POSTECH
- 项目：[plemeri/InSPyReNet](https://github.com/plemeri/InSPyReNet) · [transparent-background](https://github.com/plemeri/transparent-background)
- 论文：*Revisiting Image Pyramid Structure for High Resolution Salient Object Detection*（ACCV 2022）
- 许可证：MIT
- 用途：通用显著目标抠图（`model_fp16.onnx`）

### u²-Net · 轻量显著性检测

- 作者：Xuebin Qin（秦雪宾）等
- 项目：[xuebinqin/U-2-Net](https://github.com/xuebinqin/U-2-Net)
- 论文：*U²-Net: Going Deeper with Nested U-Structure for Salient Object Detection*（Pattern Recognition 2020）
- 许可证：Apache-2.0
- 用途：轻量 CPU 档通用抠图，秒级出图（`u2netp.onnx`）

### YuNet · 人脸检测

- 作者：Shiqi Yu（于仕琪）团队（libfacedetection）· OpenCV Zoo 维护团队
- 项目：[opencv/opencv_zoo](https://github.com/opencv/opencv_zoo) · [libfacedetection](https://github.com/ShiqiYu/libfacedetection)
- 许可证：Apache-2.0（OpenCV Zoo）· MIT（libfacedetection）
- 用途：人脸检测，证件照头部定位与构图校验（`face_detection_yunet_2023mar.onnx`）

### 开发框架与工具

- [Rust](https://www.rust-lang.org/)（MIT/Apache-2.0）与 [iced](https://github.com/iced-rs/iced)（MIT）GUI 框架
- [ONNX Runtime](https://github.com/microsoft/onnxruntime)（MIT）与 [ort](https://github.com/pykeio/ort)（MIT/Apache-2.0）推理后端，DirectML 加速
- [windows-rs](https://github.com/microsoft/windows-rs)（MIT/Apache-2.0）、[notify-rust](https://github.com/hoodie/notify-rust)（MIT/Apache-2.0）
- [NSIS](https://nsis.sourceforge.io/)（zlib 许可）安装包制作

### 支持项目

壹鸣是**免费 · 无广告 · 本地隐私**的桌面工具：所有功能完全免费，图片处理全程留在你的电脑上，不上传任何数据。如果它帮到了你，欢迎以任意一种方式支持持续开发：

- 点一下**标题栏右上角的 ☕**，请作者喝杯咖啡——¥6.6 起，金额随心就好；
- 或者花 **0 元**给项目一个 **Star**，同样是很好的支持；
- 提 Issue、报 Bug、写教程、推荐给朋友，都算。

> 打赏不升级、不解锁、不影响任何功能——你的每一杯咖啡，都会变成下一个新功能。

### 支持者

感谢每一位通过打赏、反馈与传播支持壹鸣的朋友——是你们让「免费 · 无广告 · 本地隐私」可以一直持续下去。

*（此处后续将展示每月支持者名单，数据来自爱发电，敬请期待。）*

