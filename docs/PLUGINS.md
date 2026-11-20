# Atlas Forge 本地插件编写说明

插件是一份带版本的 JSON 清单，定义可重用的地图命令。桌面和命令行共用同一套草稿 API。可以用 Python、PowerShell 或其他工具生成清单和参数文件。

## 运行示例

1. 从 **工具 → 本地插件…** 打开 `examples/create-city.atlas-plugin.json`。便携包把示例放在 `示例\create-city.atlas-plugin.json`。
2. 选择「放置城市」，填写名称和经纬度，点「生成草稿并检查结果」。
3. 检查修改，再点「应用到地图」。一次应用对应一次撤销。读图命令的结果可直接展开查看。

在项目源码目录也可运行：

```powershell
& .\target\release\atlas-script.exe --blank --plugin .\docs\examples\create-city.atlas-plugin.json --command create-city --out .\plugin-world.atlas
```

`--params-file args.json` 可传入 `{"name":"海岬","lon":100,"lat":20}`。省略有默认值的参数时沿用清单默认值。

## 清单格式 1

- 根对象含 `schema: 1`、`id`、`name`、`version` 和非空的 `commands` 数组。ID 使用字母、数字、点、短横线或下划线。同一插件的命令 ID 必须唯一。
- 命令含 `id`、`label`、可选 `description`、可选 `parameters` 和非空的 `requests` 数组。请求按顺序提交，后续查询可以读取前面生成的草稿。
- `parameters` 由参数名映射到 `label`、`type` 和可选的 `default`。可用类型为 `string`、`integer`、`number`、`boolean` 和 `json`。缺少默认值时，需要填写相应参数。
- 用 `{"$param":"lon"}` 引用参数，整个 JSON 对象替换为带原始类型的值。例如经度会是数字。可在 `args` 和 `context` 中引用参数，`op` 使用固定操作名。
- `atlas-script --blank --request '{"op":"automation.capabilities"}'` 可列出操作名与参数。插件常用 `edit.create`、`edit.rename`、`edit.draw_land`、`edit.transfer`、`world.summary` 和 `entities.list`。[脚本和 MCP 契约](API.md)

## 执行边界与扩展

格式 1 的扩展点是地图请求序列。加载清单只解析 JSON，运行调用当前进程中的地图操作。一个步骤失败时整批回滚。桌面由用户确认应用，命令行由调用方用 `--out` 另存。需要导出图片时，在事务成功后通过脚本 API 发起 `preview.render`。该契约为以后增加扩展类型保留了独立的 `schema` 版本。
