# 程序图标

图标的源文件是手写的矢量代码（SVG）：

- `atlas-forge.svg`：40 px 及以上。黑色圆角底、石墨色地球与银色经纬网、香槟金的大陆（被共享国界分成两国）与岛、倾斜的金色环（atlas）和一颗星芒（forge）。
- `atlas-forge-small.svg`：16–32 px 用的简化版（不画经纬网和星芒，线条加粗，加一圈浅色描边，在深色任务栏上也看得清）。

由它们生成、供程序使用的文件：

- `atlas-forge.ico`：Windows 程序图标（16–256 px），由 `crates/map_app/build.rs` 编进 atlas.exe。
- `atlas-forge-256.png`：窗口与任务栏图标，程序启动时载入（`main.rs` 的 `with_app_icon`）。
- `atlas-forge-64.png`：创作工作室顶栏和开始卡片中的图标。
- `atlas-forge-1024.png`：大图。

修改图标：编辑两个 SVG，运行 `python tools/make_icon.py`（需要 Pillow 和 `resvg-py` 或 `cairosvg`）重新生成，再重新编译。
