# Atlas Forge user guide

Atlas Forge is a desktop program for drawing fictional worlds and historical maps: land and countries on one map whose borders, coasts and ownership change year by year, exported as finished maps. This guide covers the usual ways of working. Every command is also described in the program: press **Ctrl+K** (command palette) or **F1** (shortcuts).

The interface language is set in Preferences (中文 / English); parts of the interface not yet translated stay in Chinese.

## 1. Installing and starting

- Unzip `AtlasForge-<version>-windows-x64.zip` and run `atlas.exe`; nothing is installed.
- A graphics card with DirectX 12, Vulkan or OpenGL is needed. If the program cannot start it says so: update the graphics driver; remote desktops and virtual machines may lack a usable device; or set the environment variable `WGPU_BACKEND=dx12` (or `vulkan`, `gl`) and start again.
- The default font of the interface and the exports is the bundled Noto Sans SC (the `fonts` folder, SIL Open Font License); without that folder a Chinese system font is used (Microsoft YaHei, SimHei, …). Names on the map can also use any font installed on the system.
- Checking the program: `atlas.exe --self-test <folder>` makes a world, partitions a country, builds roads, saves and reads back, exports PNG/SVG/PDF, and writes its report to `自检报告.txt` in that folder.
- Opening a project: drop the `.atlas` file on `atlas.exe`, or run `atlas.exe world.atlas`.

## 2. The window

- **Tools (left)**: select, draw land, expand, take, carve lakes, assign, split, lines and rivers, labels, symbols, measure, …
- **Options bar (top)**: the current tool's options.
- **Centre**: the flat map and the globe (View menu to show one). Wheel to zoom; hold Space and drag to pan in any tool.
- **Panels (right)**: countries, data table, fields, thematic maps, styles, layers, reference images; the selection's properties.
- **Timeline (bottom)**: when on, the map shows the chosen year and edits take effect from that year on; when off, the latest state.

## 3. Starting from nothing: the new-world wizard

File → New world wizard…: planet radius, land share, continents, roughness, number of countries, name style, rivers, capitals, an optional random history, and a seed (the same seed gives the same world). Everything generated stays editable.

## 4. Drawing

- **Draw land / carve water**, **expand** (unclaimed land and sea only) or **take** (neighbours' land too) for the selected country, **assign** a face, **fill bucket**, **split** and **merge**.
- Snapping to edges, nodes and guides while drawing (hold Ctrl to turn it off for a moment).
- Undo / redo with Ctrl+Z / Ctrl+Y; the History panel jumps to any step.

## 5. Years and history

- Turn the timeline on (Ctrl+T) and pick a year before editing: the change applies from that year on; earlier years stay as they were.
- Countries have founding and ending years and successors; parents (a province under a country) can change by year too.
- **Events** (Time → Event timeline…), **local views** (a region and a span of years, played back), **custom calendars** (how years are shown).

## 6. Countries, regions and names

- **Automatic partition**: select a country → "自动划分…" in its properties: into new countries or into its provinces, about equal in area, along rivers and ridges where possible; previewed, applied as one undo step. Lakes and enclaves inside are left alone.
- **Name generator** (Tools → Name generator…) and the "random" button by a name; **name styles** per unit, lent by a culture to the places it covers; **automatic colours** (neighbours differ).

## 7. Layers and data

Culture or religion layers beside the political one; share layers split a face between units by percentage. Data fields (population, …) change by year and drive thematic maps. CSV, GeoJSON and Shapefile import and export.

## 8. Routes, roads and travel

- Routes: campaigns, trade routes, sea lanes, railways; arrows, dashes, broad arrows, decorations (front teeth, railway ticks, peaks, trees).
- **Road network** (Tools → Roads and travel…): roads between the cities around seas and lakes, steep ground dearer; previewed, applied as routes.
- **Travel**: from one city to another along the routes, with the days on foot, by caravan, on horseback and by relay horses; the measuring tool and a selected route show days too.

## 9. Look

Map themes (modern, classic atlas, parchment, dark), named styles and swatches, area patterns (stripes, cross, dots, mountains, forest), hill shading from terrain, country names bent along the land and draped on the globe.

## 10. Export and layout

- File → Export… (Ctrl+E): PNG, SVG, PDF, GeoJSON, Shapefile, CSV; one file per year, animations (with ffmpeg), atlases from saved layouts.
- **Layout view** (File → Layout view…): the page exactly as exported; drag the title, legend, scale bar and north arrow; add inset maps, move them, resize them by their corner. Positions are kept with the export settings and saved layouts.

## 11. Saving and recovery

Projects are `.atlas` files. Each save keeps the file it replaces as `.atlas.bak1` (older ones `.bak2`, `.bak3`). After a crash the next start offers to recover unsaved work.

## 12. Interfaces for scripts and AI

`atlas-query.exe` queries a project from the command line (JSON); `atlas-mcp.exe` is an MCP server for AI clients. The in-app AI panel (when configured) works on a draft that is previewed before it is applied.

## 13. Planetary environment and climate

Open **Tools → Planetary environment and climate…**, then select **运行模拟** to calculate one reference or **实时更新** to update it as the geography changes. The climate panel uses Chinese labels. It includes month and season selectors, a point inspector, editable and lockable fields, saved scenarios and PNG/CSV export. Visible climate overlays are included in map exports. The physical grid has 96 × 48 cells (3.75 degrees). Projects with climate data require a reader supporting file format 2. [Climate guide in Chinese](CLIMATE.md)
