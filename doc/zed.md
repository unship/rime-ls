# Zed 配置示例

> **Warning**
> 只在最新版 Zed (v0.133.7) 的 linux 版本测试过。

Zed 添加 LSP 服务需要写插件，可以参考 [zed-ext-rime-ls](https://github.com/wlh320/zed-ext-rime-ls) 的实现。

使用方法是 `git clone` 下来，通过 Install Dev Extension 安装。将启动命令设为 `rime_ls --connect`，若服务端未运行，客户端会自动在后台启动。配置项在 `~/.config/rime-ls/config.yaml` 中设置。
