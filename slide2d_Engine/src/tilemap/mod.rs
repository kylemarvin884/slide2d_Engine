//! 瓦片集、瓦片地图和编辑工具共用数据。

use crate::localization::tr;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

/// 一个图集切分后的单类瓦片属性。
#[derive(Clone, Serialize, Deserialize)]
pub struct TileProperty {
    pub tile_id: u32,
    pub collision: bool,
    pub transparent: bool,
}

/// 瓦片集资源，保存为assets/tilesets目录中的JSON文件。
#[derive(Clone, Serialize, Deserialize)]
pub struct TileSet {
    /// Slide2D瓦片集JSON内置格式标识。
    #[serde(default = "tileset_format")]
    pub slide2d_engine: String,
    pub name: String,
    pub image_path: String,
    pub tile_width: u32,
    pub tile_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub properties: Vec<TileProperty>,
}

impl TileSet {
    /// 根据图集尺寸和统一格子尺寸创建瓦片集。
    pub fn new(
        name: String,
        image_path: String,
        image_width: u32,
        image_height: u32,
        tile_width: u32,
        tile_height: u32,
    ) -> Self {
        let safe_width = tile_width.max(1);
        let safe_height = tile_height.max(1);
        let columns = image_width / safe_width;
        let rows = image_height / safe_height;
        let mut properties = Vec::new();
        for tile_id in 0..columns * rows {
            properties.push(TileProperty {
                tile_id,
                collision: false,
                transparent: false,
            });
        }
        Self {
            slide2d_engine: tileset_format(),
            name,
            image_path,
            tile_width: safe_width,
            tile_height: safe_height,
            columns,
            rows,
            properties,
        }
    }

    /// 从JSON读取瓦片集。
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|error| format!("读取瓦片集失败：{error}"))?;
        serde_json::from_str(&text).map_err(|error| format!("解析瓦片集失败：{error}"))
    }

    /// 保存瓦片集JSON。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("生成瓦片集JSON失败：{error}"))?;
        std::fs::write(path, text).map_err(|error| format!("保存瓦片集失败：{error}"))
    }

    /// 返回指定瓦片属性。
    pub fn property(&self, tile_id: u32) -> Option<&TileProperty> {
        self.properties
            .iter()
            .find(|property| property.tile_id == tile_id)
    }
}

/// 返回瓦片集资源固定的Slide2D格式标识。
fn tileset_format() -> String {
    "SLIDE2D_TILESET".to_owned()
}

/// 瓦片地图的固定三层。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileLayerKind {
    Ground,
    Decoration,
    Collision,
}

impl TileLayerKind {
    /// 返回用于界面显示的中文层名称。
    pub fn display_name(self) -> String {
        match self {
            Self::Ground => tr("tile.ground"),
            Self::Decoration => tr("tile.decoration"),
            Self::Collision => tr("tile.collision"),
        }
    }
}

/// 一个已绘制的瓦片格子。
#[derive(Clone, Serialize, Deserialize)]
pub struct TileCell {
    pub x: i32,
    pub y: i32,
    pub tile_id: u32,
}

/// 一层稀疏瓦片数据，只保存真正绘制过的格子。
#[derive(Serialize, Deserialize)]
pub struct TileLayer {
    pub kind: TileLayerKind,
    pub visible: bool,
    pub cells: Vec<TileCell>,
    #[serde(skip)]
    cell_index: RefCell<Option<TileCellIndex>>,
}

/// 不参与序列化的坐标索引；记录长度用于发现外部对公开cells的clear等修改。
struct TileCellIndex {
    positions: HashMap<(i32, i32), usize>,
    cells_len: usize,
}

impl Clone for TileLayer {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            visible: self.visible,
            cells: self.cells.clone(),
            cell_index: RefCell::new(None),
        }
    }
}

impl TileLayer {
    /// 创建空瓦片层。
    pub fn new(kind: TileLayerKind) -> Self {
        Self {
            kind,
            visible: true,
            cells: Vec::new(),
            cell_index: RefCell::new(None),
        }
    }

    /// 查询指定格子的瓦片ID。
    pub fn tile_at(&self, x: i32, y: i32) -> Option<u32> {
        self.ensure_cell_index();
        let index = self
            .cell_index
            .borrow()
            .as_ref()
            .and_then(|index| index.positions.get(&(x, y)).copied());
        index.and_then(|index| self.cells.get(index).map(|cell| cell.tile_id))
    }

    /// 绘制或覆盖一个瓦片。
    pub fn set_tile(&mut self, x: i32, y: i32, tile_id: u32) {
        self.ensure_cell_index();
        let existing = self
            .cell_index
            .borrow()
            .as_ref()
            .and_then(|index| index.positions.get(&(x, y)).copied());
        if let Some(index) = existing {
            self.cells[index].tile_id = tile_id;
        } else {
            let index = self.cells.len();
            self.cells.push(TileCell { x, y, tile_id });
            if let Some(cell_index) = self.cell_index.get_mut() {
                cell_index.positions.insert((x, y), index);
                cell_index.cells_len = self.cells.len();
            }
        }
    }

    /// 删除指定格子的瓦片。
    pub fn erase_tile(&mut self, x: i32, y: i32) {
        self.ensure_cell_index();
        let removed = self
            .cell_index
            .get_mut()
            .as_mut()
            .and_then(|index| index.positions.remove(&(x, y)));
        if let Some(removed) = removed {
            self.cells.swap_remove(removed);
            if removed < self.cells.len() {
                let moved = &self.cells[removed];
                if let Some(index) = self.cell_index.get_mut() {
                    index.positions.insert((moved.x, moved.y), removed);
                }
            }
            if let Some(index) = self.cell_index.get_mut() {
                index.cells_len = self.cells.len();
            }
        }
    }

    /// 使用同一瓦片替换闭区间矩形，复杂度为O(现有格子数+矩形面积)。
    pub fn replace_rect(
        &mut self,
        minimum_x: i32,
        minimum_y: i32,
        maximum_x: i32,
        maximum_y: i32,
        tile_id: u32,
    ) {
        if minimum_x > maximum_x || minimum_y > maximum_y {
            return;
        }
        self.ensure_cell_index();
        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                self.set_tile(x, y, tile_id);
            }
        }
    }

    /// 丢弃当前层内容并使用同一瓦片填满从(0, 0)开始的有限范围。
    pub fn fill_entire(&mut self, width: u32, height: u32, tile_id: u32) {
        let width = width.min(i32::MAX as u32);
        let height = height.min(i32::MAX as u32);
        let cell_count = (width as usize).checked_mul(height as usize).unwrap_or(0);
        let mut cells = Vec::with_capacity(cell_count);
        let mut positions = HashMap::with_capacity(cell_count);
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                positions.insert((x, y), cells.len());
                cells.push(TileCell { x, y, tile_id });
            }
        }
        self.cells = cells;
        *self.cell_index.get_mut() = Some(TileCellIndex {
            positions,
            cells_len: cell_count,
        });
    }

    /// 从起点进行四方向洪水填充，最多处理10000格防止误操作无限扩张。
    pub fn fill(&mut self, start_x: i32, start_y: i32, tile_id: u32) {
        let old_tile = self.tile_at(start_x, start_y);
        if old_tile == Some(tile_id) {
            return;
        }
        let mut queue = VecDeque::from([(start_x, start_y)]);
        let mut visited = HashSet::new();
        while let Some((x, y)) = queue.pop_front() {
            if visited.len() >= 10_000 || !visited.insert((x, y)) {
                continue;
            }
            if self.tile_at(x, y) != old_tile {
                continue;
            }
            self.set_tile(x, y, tile_id);
            if let Some(next_x) = x.checked_add(1) {
                queue.push_back((next_x, y));
            }
            if let Some(next_x) = x.checked_sub(1) {
                queue.push_back((next_x, y));
            }
            if let Some(next_y) = y.checked_add(1) {
                queue.push_back((x, next_y));
            }
            if let Some(next_y) = y.checked_sub(1) {
                queue.push_back((x, next_y));
            }
        }
    }

    /// 首次查询、反序列化、Clone或外部清空cells后按需重建坐标索引。
    fn ensure_cell_index(&self) {
        let needs_rebuild = self
            .cell_index
            .borrow()
            .as_ref()
            .map(|index| index.cells_len != self.cells.len())
            .unwrap_or(true);
        if !needs_rebuild {
            return;
        }
        let mut positions = HashMap::with_capacity(self.cells.len());
        for (index, cell) in self.cells.iter().enumerate() {
            positions.entry((cell.x, cell.y)).or_insert(index);
        }
        *self.cell_index.borrow_mut() = Some(TileCellIndex {
            positions,
            cells_len: self.cells.len(),
        });
    }
}

/// 场景中的完整瓦片地图。
#[derive(Clone, Serialize, Deserialize)]
pub struct TileMap {
    pub tileset_path: String,
    /// 场景内嵌瓦片集副本，确保碰撞和透明属性随scenes.json保存。
    #[serde(default)]
    pub tileset: Option<TileSet>,
    pub tile_width: u32,
    pub tile_height: u32,
    #[serde(default = "default_map_width")]
    pub map_width: u32,
    #[serde(default = "default_map_height")]
    pub map_height: u32,
    pub layers: Vec<TileLayer>,
}

impl TileMap {
    /// 创建拥有地面、装饰、碰撞三层的空地图。
    pub fn new() -> Self {
        Self {
            tileset_path: String::new(),
            tileset: None,
            tile_width: 32,
            tile_height: 32,
            map_width: default_map_width(),
            map_height: default_map_height(),
            layers: vec![
                TileLayer::new(TileLayerKind::Ground),
                TileLayer::new(TileLayerKind::Decoration),
                TileLayer::new(TileLayerKind::Collision),
            ],
        }
    }

    /// 返回指定类型瓦片层。
    pub fn layer(&self, kind: TileLayerKind) -> Option<&TileLayer> {
        self.layers.iter().find(|layer| layer.kind == kind)
    }

    /// 返回指定类型瓦片层的可变引用。
    pub fn layer_mut(&mut self, kind: TileLayerKind) -> Option<&mut TileLayer> {
        self.layers.iter_mut().find(|layer| layer.kind == kind)
    }
}

/// 瓦片绘制工具。
#[derive(Clone, Copy, PartialEq)]
pub enum TileTool {
    Select,
    Brush,
    Eraser,
    Fill,
    Rectangle,
}

/// 编辑器瓦片工具面板状态。
pub struct TileEditorState {
    pub window_open: bool,
    pub tool: TileTool,
    pub active_layer: TileLayerKind,
    pub selected_tile_id: u32,
    pub selected_tileset: Option<TileSet>,
    pub selected_tileset_path: Option<std::path::PathBuf>,
    pub new_tile_width: u32,
    pub new_tile_height: u32,
    pub rectangle_start: Option<(i32, i32)>,
}

impl TileEditorState {
    /// 创建默认瓦片工具状态。
    pub fn new() -> Self {
        Self {
            window_open: false,
            tool: TileTool::Select,
            active_layer: TileLayerKind::Ground,
            selected_tile_id: 0,
            selected_tileset: None,
            selected_tileset_path: None,
            new_tile_width: 32,
            new_tile_height: 32,
            rectangle_start: None,
        }
    }
}

/// 默认地图宽度为100格。
fn default_map_width() -> u32 {
    100
}

/// 默认地图高度为100格。
fn default_map_height() -> u32 {
    100
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证绘制、覆盖和擦除瓦片。
    #[test]
    fn tile_layer_can_paint_and_erase() {
        let mut layer = TileLayer::new(TileLayerKind::Ground);
        layer.set_tile(2, 3, 5);
        assert_eq!(layer.tile_at(2, 3), Some(5));
        layer.set_tile(2, 3, 7);
        assert_eq!(layer.tile_at(2, 3), Some(7));
        layer.erase_tile(2, 3);
        assert_eq!(layer.tile_at(2, 3), None);
    }

    /// 验证三层瓦片地图可以通过JSON保存和恢复。
    #[test]
    fn tile_map_json_preserves_layers() {
        let mut map = TileMap::new();
        map.tileset_path = "assets/tilesets/terrain.s2tileset".to_owned();
        let mut tileset = TileSet::new(
            "terrain".to_owned(),
            "assets/images/terrain.png".to_owned(),
            64,
            64,
            32,
            32,
        );
        tileset.properties[2].collision = true;
        map.tileset = Some(tileset);
        map.layer_mut(TileLayerKind::Collision)
            .unwrap()
            .set_tile(4, 5, 2);
        let json = serde_json::to_string(&map).unwrap();
        let restored: TileMap = serde_json::from_str(&json).unwrap();

        assert_eq!(
            restored
                .layer(TileLayerKind::Collision)
                .unwrap()
                .tile_at(4, 5),
            Some(2)
        );
        assert!(restored.tileset.unwrap().property(2).unwrap().collision);
    }

    /// 验证大矩形批量替换保持唯一格子和查询索引一致。
    #[test]
    fn large_rect_replace_keeps_cells_and_index_consistent() {
        let mut layer = TileLayer::new(TileLayerKind::Ground);
        layer.fill_entire(400, 300, 1);
        layer.replace_rect(100, 50, 299, 249, 7);

        assert_eq!(layer.cells.len(), 120_000);
        assert_eq!(layer.tile_at(99, 50), Some(1));
        assert_eq!(layer.tile_at(100, 50), Some(7));
        assert_eq!(layer.tile_at(299, 249), Some(7));
        assert_eq!(layer.tile_at(300, 249), Some(1));
        let unique: HashSet<_> = layer.cells.iter().map(|cell| (cell.x, cell.y)).collect();
        assert_eq!(unique.len(), layer.cells.len());
    }

    /// 验证Clone和反序列化后会惰性恢复索引，且JSON仍只有原有字段。
    #[test]
    fn cloned_and_deserialized_layers_rebuild_runtime_index() {
        let mut layer = TileLayer::new(TileLayerKind::Decoration);
        layer.fill_entire(128, 128, 3);
        let mut cloned = layer.clone();
        cloned.set_tile(127, 127, 9);
        assert_eq!(cloned.tile_at(127, 127), Some(9));
        assert_eq!(layer.tile_at(127, 127), Some(3));

        let json = serde_json::to_value(&layer).unwrap();
        assert!(json.get("cell_index").is_none());
        assert!(json.get("cells").unwrap().is_array());
        let mut restored: TileLayer = serde_json::from_value(json).unwrap();
        assert_eq!(restored.tile_at(64, 64), Some(3));
        restored.erase_tile(64, 64);
        assert_eq!(restored.tile_at(64, 64), None);
        assert_eq!(restored.cells.len(), 128 * 128 - 1);
    }

    /// 验证洪水填充只替换连通区域并保持索引与存储一致。
    #[test]
    fn large_flood_fill_is_consistent() {
        let mut layer = TileLayer::new(TileLayerKind::Ground);
        layer.fill_entire(100, 100, 2);
        layer.replace_rect(50, 0, 50, 99, 8);
        layer.fill(0, 0, 5);

        assert_eq!(layer.tile_at(49, 99), Some(5));
        assert_eq!(layer.tile_at(50, 99), Some(8));
        assert_eq!(layer.tile_at(51, 99), Some(2));
        assert_eq!(layer.cells.iter().filter(|cell| cell.tile_id == 5).count(), 5_000);
    }
}
