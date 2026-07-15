//! Slide2D声明式插件系统，只读取配置并执行引擎白名单能力，不加载脚本或动态代码。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// plugin.json必须携带的固定Slide2D Plugin System标识。
pub const PLUGIN_MAGIC: &str = "SLIDE2D_PLUGIN_SYSTEM";
/// 官方道具拾取插件的稳定ID。
pub const OFFICIAL_PICKUP_PLUGIN_ID: &str = "slide2d.official.pickup";

/// 插件节点在蓝图中的分类。
#[derive(Clone, Copy, Debug, Eq, Hash, Serialize, Deserialize, PartialEq)]
pub enum PluginNodeCategory {
    Event,
    Logic,
    Action,
    Variable,
}

/// 插件节点可选择的Runtime白名单行为。
#[derive(Clone, Debug, Eq, Hash, Serialize, Deserialize, PartialEq)]
pub enum PluginBehavior {
    /// 场景加载完成时触发的插件事件。
    SceneLoadedEvent,
    /// 当前物体被点击时触发的插件事件。
    ObjectClickedEvent,
    /// 判断当前Actor是否在本帧被鼠标拾取，并从成功或失败端口继续。
    PickupCheck,
    /// 将固定值写入当前Actor变量。
    SetObjectVariable,
    /// 将固定值写入全局变量。
    SetGlobalVariable,
    /// 按每秒速度移动当前Actor，由现有Rapier物理桥接处理碰撞。
    MoveHorizontal,
    /// 声明并初始化当前Actor数值变量。
    NumberVariable,
}

/// plugin.json声明的一个蓝图节点。
#[derive(Clone, Serialize, Deserialize)]
pub struct PluginNodeDefinition {
    pub node_type: String,
    pub display_name: String,
    pub description: String,
    pub category: PluginNodeCategory,
    pub behavior: PluginBehavior,
    #[serde(default = "default_variable_name")]
    pub variable_name: String,
    #[serde(default)]
    pub value: f32,
}

/// 插件注册的自定义资源类型。
#[derive(Clone, Serialize, Deserialize)]
pub struct PluginResourceDefinition {
    pub display_name: String,
    pub folder: String,
    pub extensions: Vec<String>,
}

/// 插件注册的声明式编辑器工具。
#[derive(Clone, Serialize, Deserialize)]
pub struct PluginEditorToolDefinition {
    pub tool_id: String,
    pub display_name: String,
    pub description: String,
}

/// 每个独立插件文件夹中的plugin.json完整结构。
#[derive(Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub slide2d_plugin_system: String,
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    #[serde(default)]
    pub nodes: Vec<PluginNodeDefinition>,
    #[serde(default)]
    pub resources: Vec<PluginResourceDefinition>,
    #[serde(default)]
    pub editor_tools: Vec<PluginEditorToolDefinition>,
    #[serde(default)]
    pub runtime_capabilities: Vec<PluginBehavior>,
}

/// 已安装插件及其本地目录。
#[derive(Clone)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub directory: PathBuf,
    pub enabled: bool,
    pub load_error: Option<String>,
}

/// 编辑器和蓝图面板共同读取的插件注册表。
#[derive(Clone)]
pub struct PluginRegistry {
    pub project_root: PathBuf,
    pub installed: Vec<InstalledPlugin>,
}

/// 单个启用插件允许Runtime执行的节点和白名单行为。
#[derive(Clone, Default)]
pub struct PluginRuntimeAuthorization {
    capabilities: HashSet<String>,
    nodes: HashMap<String, String>,
}

/// 由已验证PluginRegistry构建、供蓝图VM使用的可信授权映射。
pub type PluginAuthorizationMap = HashMap<String, PluginRuntimeAuthorization>;

impl PluginRuntimeAuthorization {
    /// 只有节点存在、蓝图behavior与manifest一致且能力已声明时才授权。
    pub fn allows(&self, node_type: &str, behavior: &PluginBehavior) -> bool {
        let Ok(serialized_behavior) = serde_json::to_string(behavior) else {
            return false;
        };
        self.nodes.get(node_type) == Some(&serialized_behavior)
            && self.capabilities.contains(&serialized_behavior)
    }
}

impl PluginRegistry {
    /// 创建新工程注册表，默认启用官方道具拾取示例插件。
    pub fn load_new_project(project_root: PathBuf) -> Self {
        let enabled = HashSet::from([OFFICIAL_PICKUP_PLUGIN_ID.to_owned()]);
        Self::load(project_root, &enabled)
    }

    /// 扫描项目plugins目录，并按工程保存的ID集合恢复启用状态。
    pub fn load(project_root: PathBuf, enabled_ids: &HashSet<String>) -> Self {
        Self::scan(project_root, Some(enabled_ids), true)
    }

    /// 裸场景只扫描相邻的现有plugins目录，不创建或隐式安装任何插件。
    pub fn load_adjacent(project_root: PathBuf) -> Self {
        Self::scan(project_root, None, false)
    }

    fn scan(
        project_root: PathBuf,
        enabled_ids: Option<&HashSet<String>>,
        install_official: bool,
    ) -> Self {
        let plugins_root = project_root.join("plugins");
        if install_official {
            let _ = fs::create_dir_all(&plugins_root);
            let _ = install_official_pickup_plugin(&plugins_root);
        }
        let mut installed = Vec::new();
        if let Ok(entries) = fs::read_dir(&plugins_root) {
            for entry in entries.flatten() {
                let directory = entry.path();
                if !directory.is_dir() {
                    continue;
                }
                let manifest_path = directory.join("plugin.json");
                match load_manifest(&manifest_path) {
                    Ok(manifest) => {
                        let enabled = enabled_ids
                            .map(|ids| ids.contains(&manifest.plugin_id))
                            .unwrap_or(true);
                        installed.push(InstalledPlugin {
                            manifest,
                            directory,
                            enabled,
                            load_error: None,
                        });
                    }
                    Err(error) => installed.push(InstalledPlugin {
                        manifest: invalid_manifest(&directory),
                        directory,
                        enabled: false,
                        load_error: Some(error),
                    }),
                }
            }
        }
        let mut plugin_id_counts = HashMap::new();
        for plugin in installed
            .iter()
            .filter(|plugin| plugin.load_error.is_none())
        {
            *plugin_id_counts
                .entry(plugin.manifest.plugin_id.clone())
                .or_insert(0_usize) += 1;
        }
        for plugin in &mut installed {
            if plugin.load_error.is_none()
                && plugin_id_counts.get(&plugin.manifest.plugin_id).copied() != Some(1)
            {
                plugin.enabled = false;
                plugin.load_error = Some(format!(
                    "发现重复插件ID：{}",
                    plugin.manifest.plugin_id
                ));
            }
        }
        installed.sort_by(|left, right| left.manifest.name.cmp(&right.manifest.name));
        Self {
            project_root,
            installed,
        }
    }

    /// 返回当前启用插件ID集合，用于工程保存和Runtime过滤。
    pub fn enabled_ids(&self) -> HashSet<String> {
        self.installed
            .iter()
            .filter(|plugin| plugin.enabled && plugin.load_error.is_none())
            .map(|plugin| plugin.manifest.plugin_id.clone())
            .collect()
    }

    /// 返回全部已启用插件节点及所属插件ID。
    pub fn enabled_nodes(&self) -> Vec<(String, PluginNodeDefinition)> {
        let mut nodes = Vec::new();
        for plugin in self
            .installed
            .iter()
            .filter(|plugin| plugin.enabled && plugin.load_error.is_none())
        {
            for node in &plugin.manifest.nodes {
                nodes.push((plugin.manifest.plugin_id.clone(), node.clone()));
            }
        }
        nodes
    }

    /// 从已验证且启用的manifest构建Runtime唯一可信的插件节点授权。
    pub fn runtime_authorizations(&self) -> PluginAuthorizationMap {
        let mut authorizations = HashMap::new();
        for plugin in self
            .installed
            .iter()
            .filter(|plugin| plugin.enabled && plugin.load_error.is_none())
        {
            let capabilities = plugin
                .manifest
                .runtime_capabilities
                .iter()
                .filter_map(|behavior| serde_json::to_string(behavior).ok())
                .collect();
            let nodes = plugin
                .manifest
                .nodes
                .iter()
                .filter_map(|node| {
                    serde_json::to_string(&node.behavior)
                        .ok()
                        .map(|behavior| (node.node_type.clone(), behavior))
                })
                .collect();
            authorizations.insert(
                plugin.manifest.plugin_id.clone(),
                PluginRuntimeAuthorization {
                    capabilities,
                    nodes,
                },
            );
        }
        authorizations
    }

    /// 切换一个插件的启用状态，下一次UI绘制和Runtime启动立即使用新状态。
    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) {
        if let Some(plugin) = self
            .installed
            .iter_mut()
            .find(|plugin| plugin.manifest.plugin_id == plugin_id && plugin.load_error.is_none())
        {
            plugin.enabled = enabled;
        }
    }

    /// 重新扫描plugins目录，同时保留当前启用ID。
    pub fn refresh(&mut self) {
        let enabled = self.enabled_ids();
        *self = Self::load(self.project_root.clone(), &enabled);
    }
}

/// 读取并严格验证plugin.json，禁止路径跳转和未知空标识。
pub fn load_manifest(path: &Path) -> Result<PluginManifest, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("读取Slide2D Plugin System配置失败：{error}"))?;
    let manifest: PluginManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("解析Slide2D Plugin System配置失败：{error}"))?;
    if manifest.slide2d_plugin_system != PLUGIN_MAGIC {
        return Err("plugin.json缺少SLIDE2D_PLUGIN_SYSTEM标识".to_owned());
    }
    if manifest.plugin_id.trim().is_empty() || manifest.name.trim().is_empty() {
        return Err("plugin.json缺少插件ID或名称".to_owned());
    }
    let mut capabilities = HashSet::new();
    for behavior in &manifest.runtime_capabilities {
        if !capabilities.insert(behavior.clone()) {
            return Err(format!("插件包含重复Runtime能力：{behavior:?}"));
        }
    }
    let mut node_types = HashSet::new();
    for node in &manifest.nodes {
        if node.node_type.trim().is_empty() {
            return Err("插件节点node_type不能为空".to_owned());
        }
        if !node_types.insert(node.node_type.clone()) {
            return Err(format!("插件包含重复node_type：{}", node.node_type));
        }
        if !capabilities.contains(&node.behavior) {
            return Err(format!(
                "插件节点{}的behavior未列入runtime_capabilities",
                node.node_type
            ));
        }
    }
    for resource in &manifest.resources {
        let folder = Path::new(&resource.folder);
        if folder.is_absolute()
            || folder
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(format!("插件资源目录不安全：{}", resource.folder));
        }
    }
    Ok(manifest)
}

/// 将外部插件文件夹递归导入当前工程plugins目录。
pub fn import_plugin_folder(source: &Path, project_root: &Path) -> Result<PathBuf, String> {
    let manifest = load_manifest(&source.join("plugin.json"))?;
    let target = project_root.join("plugins").join(&manifest.plugin_id);
    if target.exists() {
        return Err("同ID插件已经安装，请先删除旧插件".to_owned());
    }
    copy_directory(source, &target)?;
    Ok(target)
}

/// 删除指定本地插件目录，官方插件也可删除但下次新工程初始化会重新提供示例。
pub fn delete_plugin(directory: &Path, project_root: &Path) -> Result<(), String> {
    let plugins_root = project_root.join("plugins");
    if !directory.starts_with(&plugins_root) {
        return Err("拒绝删除plugins目录外的文件".to_owned());
    }
    fs::remove_dir_all(directory).map_err(|error| format!("删除本地插件失败：{error}"))
}

/// 自动创建官方“道具拾取系统”示例插件及带品牌标识的plugin.json。
fn install_official_pickup_plugin(plugins_root: &Path) -> Result<(), String> {
    let directory = plugins_root.join("slide2d_official_pickup");
    let path = directory.join("plugin.json");
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(&directory).map_err(|error| format!("创建官方插件目录失败：{error}"))?;
    let manifest = PluginManifest {
        slide2d_plugin_system: PLUGIN_MAGIC.to_owned(),
        plugin_id: OFFICIAL_PICKUP_PLUGIN_ID.to_owned(),
        name: "道具拾取系统".to_owned(),
        version: "1.0.0".to_owned(),
        author: "Slide2D Official".to_owned(),
        description: "提供拾取判定蓝图节点，演示无脚本插件扩展。".to_owned(),
        nodes: vec![PluginNodeDefinition {
            node_type: "pickup_check".to_owned(),
            display_name: "拾取判定".to_owned(),
            description: "当前Actor被鼠标点击时走拾取成功端口，否则走未拾取端口。".to_owned(),
            category: PluginNodeCategory::Logic,
            behavior: PluginBehavior::PickupCheck,
            variable_name: "picked".to_owned(),
            value: 1.0,
        }],
        resources: vec![PluginResourceDefinition {
            display_name: "拾取道具资源".to_owned(),
            folder: "plugins/slide2d_official_pickup/items".to_owned(),
            extensions: vec!["s2item".to_owned()],
        }],
        editor_tools: vec![PluginEditorToolDefinition {
            tool_id: "pickup_help".to_owned(),
            display_name: "拾取系统工具".to_owned(),
            description: "查看拾取节点使用说明和已注册能力。".to_owned(),
        }],
        runtime_capabilities: vec![PluginBehavior::PickupCheck],
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("生成官方plugin.json失败：{error}"))?;
    fs::write(path, bytes).map_err(|error| format!("写入官方plugin.json失败：{error}"))
}

/// 递归复制插件目录，不执行其中的任何文件。
fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| format!("创建插件目录失败：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("读取插件目录失败：{error}"))?
    {
        let entry = entry.map_err(|error| format!("读取插件文件失败：{error}"))?;
        let output = target.join(entry.file_name());
        if entry.path().is_dir() {
            copy_directory(&entry.path(), &output)?;
        } else {
            fs::copy(entry.path(), output).map_err(|error| format!("复制插件文件失败：{error}"))?;
        }
    }
    Ok(())
}

/// 为缺少有效配置的目录创建只用于管理器显示的占位信息。
fn invalid_manifest(directory: &Path) -> PluginManifest {
    PluginManifest {
        slide2d_plugin_system: PLUGIN_MAGIC.to_owned(),
        plugin_id: format!("invalid:{}", directory.display()),
        name: directory
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("无效插件")
            .to_owned(),
        version: "未知".to_owned(),
        author: "未知".to_owned(),
        description: "插件配置无效".to_owned(),
        nodes: Vec::new(),
        resources: Vec::new(),
        editor_tools: Vec::new(),
        runtime_capabilities: Vec::new(),
    }
}

/// 插件节点未声明变量名时使用简单默认值。
fn default_variable_name() -> String {
    "plugin_value".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> PluginManifest {
        PluginManifest {
            slide2d_plugin_system: PLUGIN_MAGIC.to_owned(),
            plugin_id: "test.plugin".to_owned(),
            name: "Test".to_owned(),
            version: "1".to_owned(),
            author: "Test".to_owned(),
            description: "Test".to_owned(),
            nodes: vec![PluginNodeDefinition {
                node_type: "set_value".to_owned(),
                display_name: "Set value".to_owned(),
                description: "Test".to_owned(),
                category: PluginNodeCategory::Action,
                behavior: PluginBehavior::SetObjectVariable,
                variable_name: "value".to_owned(),
                value: 1.0,
            }],
            resources: Vec::new(),
            editor_tools: Vec::new(),
            runtime_capabilities: vec![PluginBehavior::SetObjectVariable],
        }
    }

    fn write_manifest(root: &Path, manifest: &PluginManifest) -> PathBuf {
        fs::create_dir_all(root).expect("应创建插件测试目录");
        let path = root.join("plugin.json");
        fs::write(&path, serde_json::to_vec(manifest).unwrap()).expect("应写入plugin.json");
        path
    }

    fn unique_test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "slide2d_plugin_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    /// 验证官方插件会自动安装并默认启用拾取节点。
    #[test]
    fn official_pickup_plugin_is_installed() {
        let root = std::env::temp_dir().join(format!("slide2d_plugin_test_{}", std::process::id()));
        let registry = PluginRegistry::load_new_project(root.clone());
        let plugin = registry
            .installed
            .iter()
            .find(|plugin| plugin.manifest.plugin_id == OFFICIAL_PICKUP_PLUGIN_ID)
            .expect("官方拾取插件应存在");
        assert!(plugin.enabled);
        assert!(plugin
            .manifest
            .nodes
            .iter()
            .any(|node| node.behavior == PluginBehavior::PickupCheck));
        let _ = fs::remove_dir_all(root);
    }

    /// 验证plugin.json缺少Slide2D Plugin System标识时会被拒绝。
    #[test]
    fn invalid_plugin_magic_is_rejected() {
        let path = std::env::temp_dir().join("slide2d_invalid_plugin.json");
        fs::write(&path, br#"{"slide2d_plugin_system":"WRONG","plugin_id":"bad","name":"bad","version":"1","author":"x","description":"x"}"#)
            .expect("应写入测试配置");
        assert!(load_manifest(&path).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_node_and_capability_declarations_are_rejected() {
        let cases = [
            {
                let mut manifest = valid_manifest();
                manifest.nodes[0].node_type.clear();
                manifest
            },
            {
                let mut manifest = valid_manifest();
                manifest.nodes.push(manifest.nodes[0].clone());
                manifest
            },
            {
                let mut manifest = valid_manifest();
                manifest.runtime_capabilities.clear();
                manifest
            },
            {
                let mut manifest = valid_manifest();
                manifest
                    .runtime_capabilities
                    .push(PluginBehavior::SetObjectVariable);
                manifest
            },
        ];

        for (index, manifest) in cases.iter().enumerate() {
            let root = unique_test_directory(&format!("invalid_{index}"));
            let path = write_manifest(&root, manifest);
            assert!(load_manifest(&path).is_err());
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn noncompliant_installed_plugin_has_load_error() {
        let root = unique_test_directory("load_error");
        let plugin_directory = root.join("plugins/bad_plugin");
        let mut manifest = valid_manifest();
        manifest.runtime_capabilities.clear();
        write_manifest(&plugin_directory, &manifest);

        let registry = PluginRegistry::load(root.clone(), &HashSet::from([manifest.plugin_id]));
        let plugin = registry
            .installed
            .iter()
            .find(|plugin| plugin.directory == plugin_directory)
            .expect("不合规插件目录仍应显示");

        assert!(!plugin.enabled);
        assert!(plugin.load_error.is_some());
        let _ = fs::remove_dir_all(root);
    }
}
