# helix 配置示例

helix 自带 LSP 支持，只需要修改配置文件。

## 使用方法

例如为 markdown 文件启用 rime-ls，在 `~/.config/helix/languages.toml` 中增加如下配置：

### Before 23.10

```toml
[[language]]
name = "markdown"
scope = "source.markdown"
file-types = ["md", "markdown"]
language-server = { command = ["/path/to/rime-ls", "--connect"] }
```

### Since 23.10

```toml
[language-server.rime-ls]
command = ["/path/to/rime-ls", "--connect"]

[[language]]
name = "markdown"
scope = "source.markdown"
file-types = ["md", "markdown"]
language-servers = ["rime-ls"]
```

Rime 目录及补全行为（如 `long_filter_text`）在 `~/.config/rime-ls/config.yaml` 中配置。若服务端未运行，客户端会自动在后台启动。

对于 helix 上面的 LSP 的更多配置请参考 helix 的官方文档，例如怎么为所有文件开启某个 LSP server。

## 存在问题

- [x] 补全触发条件有问题(**已解决**)
  - [x] 在汉字后面输入不会自动触发补全，需在 `~/.config/rime-ls/config.yaml` 中配置 `long_filter_text: true`
  - [x] 最小补全长度为 2，手动设置最小补全长度为 1 会导致当前输入长度为 2 时补全消失(helix 最新版已无问题)
