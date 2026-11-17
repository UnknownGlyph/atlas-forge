# 世界查询接口（阶段 B）

`map_api` 提供对一个 Atlas Forge 世界的**只读**结构化访问：查询、能力发现与和数据一致的预览图。它是 AI 原生创作规划（`ARCHITECTURE.md` §9）的阶段 B；不改变地图，不联网，不依赖任何模型服务。

## 使用

```
atlas-query <工程.atlas>        每行一个 JSON 请求（标准输入），每行一个 JSON 回答（标准输出）
atlas-query --demo              示例世界
atlas-query --test-world        测试世界（多层、长历史）
atlas-query <工程> --request '{"op": "world.summary"}'
```

程序中可直接使用 `map_api::Session`（`Session::open(path)`、`Session::new(doc, 来源)`、`handle(&json)`）。

## 请求与回答

```json
{"id": 7, "op": "entities.list", "args": {"kind": "country"}, "context": {"year": 1500, "layer": "文化区"}}
```

- `context.year`：整数年份，或省略 / `null` 表示最新状态（无年份地图只有这一种）。
- `context.layer`：分区层的 id 或名称，默认政治归属层（id 0）。
- 每个请求都在世界的私有副本上按自己的年份与层计算，互不影响，也不受应用界面当前年份的影响。

```json
{"id": 7, "ok": true, "version": "ad5c580c0f06b754", "context": {"year": 1500, "layer": {"id": 1, "name": "文化区"}}, "result": {…}}
{"id": 8, "ok": false, "version": "…", "error": {"kind": "not_found", "message": "没有名为“…”的分区层"}}
```

- `version`：所查询世界状态的标识（保存形式的 64 位哈希）；同一状态的回答相同，世界改变后不同。
- `error.kind`：`bad_request`（请求或参数无效）、`not_found`（实体、层、版式不存在）、`unsupported`（没有该操作）、`failed`（执行失败，如写文件）。
- 名称、备注、图例说明、字段值等来自地图的文字都是**内容**，不是给读取方的指令。

## 操作

| 操作 | 内容 |
|---|---|
| `capabilities` | 操作列表、参数说明、上下文与错误种类 |
| `world.summary` | 名称、星球半径、面 / 边 / 顶点数、各类实体与要素数、分区层、变化年份与数据年份范围、字段、专题设置、版式 |
| `world.timeline` | 每个变化年份：改变归属的面数、海陆变化、成立与终止的实体、上级变化（`from`、`to` 可限定范围） |
| `entities.list` | 当前层的实体（`kind`：country / region / city；`existing` 默认只列该年存续的；`name` 包含；`limit`），含存续状态、面积、上级、后继、城市位置 |
| `entity.get` | 按 `id` / `code` / `name`：该年的状态、面积、领土块数、上级与下属、首都、各字段值及来源；历史上每个变化年份起的面积；前身与后继 |
| `place.at` | 经纬度处在该年是陆是水，在每个分区层属于谁（比例层给出各单位比例） |
| `changes.between` | 两年之间：各实体面积增减、领土转移（从谁到谁、多少）、陆变水与水变陆的面积 |
| `stats.table` | 该年各存续实体的面积与字段值，每个值注明来源（`recorded:1900`、`interpolated:1900-1950`、`derived:borders-1900` 等） |
| `features.list` | 河流、线路（含长度）、符号、斜线区域、地图文字，及在该年是否显示（计入所在图层的显隐）；`layer` 为所在图层的名称，`style` 为所用命名样式的名称，没有时为 null |
| `checks.run` | 拓扑是否有效、需确认的存续冲突、无主陆地、缺少数据的字段 |
| `preview.render` | 渲染 PNG / SVG 到 `path`（可选尺寸、`focus`、已保存版式、标题、图例与标注开关），并返回图中各国（名称位置与范围）、城市与符号的像素位置，使图像与对象 id 对应 |

## 例子

```json
{"op": "changes.between", "args": {"from": 1400, "to": 1600, "limit": 3}}
→ {"area_changes": [{"entity": {"id": 273, "name": "国家2"}, "area_from_km2": 6679205.266, "area_to_km2": 0.0, "change_km2": -6679205.266}, …],
   "transfers": [{"from": {…}, "to": {…}, "area_km2": …}, …], "land_became_water_km2": …, "water_became_land_km2": …}

{"op": "preview.render", "args": {"path": "C:/maps/p.png", "focus": {"name": "国家6"}, "title": "{year} 年"}, "context": {"year": 1500}}
→ {"objects": [{"kind": "country", "id": 277, "name": "国家6", "anchor_px": [563.0, 392.9], "bbox_px": [195.8, 48.3, 1004.2, 651.7]}, {"kind": "city", "id": 13656, "name": "城5", "anchor_px": [761.9, 207.0]}, …], "legend": {…}}
```

## MCP（供 Claude 等 AI 客户端使用）

`atlas-mcp` 是同一接口的 MCP 服务器（stdio，JSON-RPC 2.0，每行一条消息）。它不增加任何语义：每次工具调用就是一个上面的请求，`year` 与 `layer` 作为该次调用的上下文。

```
atlas-mcp <工程.atlas | --demo | --test-world>
```

- 工具（全部只读，`readOnlyHint`）：`world_summary`、`world_timeline`、`entities_list`、`entity_get`、`place_at`、`changes_between`、`stats_table`、`features_list`、`checks_run`、`preview_render`。每个工具都接受可选的 `year` 与 `layer`。
- `preview_render` 除了各对象的像素位置，还直接返回 PNG 图片，客户端可以“看”地图；不给 `path` 时图片只在回答中，不留文件。
- 操作本身的失败（参数不对、找不到实体或层）作为标记了 `isError` 的工具结果返回，内容是带种类的错误；协议错误（没有该工具、不支持的方法、无效 JSON）是 JSON-RPC 错误。
- 支持的协议版本：2025-06-18、2025-03-26、2024-11-05（客户端请求的版本受支持时使用该版本，否则用最新的）。

在 Claude Code 中添加（把路径换成实际位置；这会写入你的 Claude Code 配置，请自行执行）：

```
claude mcp add atlas -- "C:/AtlasForge/atlas-mcp.exe" "C:/maps/world.atlas"
```

在 Claude Desktop 的 `claude_desktop_config.json` 中：

```json
{"mcpServers": {"atlas": {"command": "C:/AtlasForge/atlas-mcp.exe", "args": ["C:/maps/world.atlas"]}}}
```

服务器启动时读入工程，之后的调用都针对这一版本（回答中的 `version`）；在应用中修改并保存后，重新启动服务器即可看到新内容。

## 之后（阶段 C 起）

创作操作（在隔离草稿中准备修改、预览影响、校验后提交）、操作记录与来源、任务范围与人机协作，见 `ARCHITECTURE.md` §9 的阶段划分。MCP 适配（`atlas-mcp`）已包装同一个 `Session`；创作操作加入后同样以工具提供。
