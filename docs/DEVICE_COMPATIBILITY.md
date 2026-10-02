# 小米穿戴设备兼容范围

桌面端面向小米手表与手环。Redmi Watch 5 是已完成现有应用验证的机型；其他设备允许尝试连接和安装，不能据此承诺界面或功能已适配。

## 安装与运行分别验证

- 设备必须属于当前登录的小米账号，且连接凭据可用。
- 当前桌面连接与 RPK 传输采用经典蓝牙 SPP。仅支持 BLE 的设备需要后续接入 BLE 传输，不能保证通过当前路径安装。
- 设备固件必须支持 Vela 快应用和第三方 RPK 安装。未知机型不会因名称或未列入目录而被禁止尝试。
- 内置 RPK 的 `deviceTypeList` 为 `watch`，没有 Redmi Watch 5 机型白名单；设计宽度为 432。其他屏幕尺寸、圆屏、字体与触控区域仍需实测。
- 应用目前使用 `@system.fetch` 与电脑 HTTP 服务通信。安装成功不代表能够收发指令。小米官方当前文档列出小米手环 8 Pro、9/9 Pro、10 与 Redmi Watch 4 未支持此接口；这些设备会显示能力提示，但仍可尝试安装。

## 名称与连接协议

云端 `model` 可能是商品名，也可能是内部标识。例如 `miwear.watch.n67cn` 对应小米手环 9 Pro，`miwear.watch.o65` 对应 Redmi Watch 5。目录采用完整型号或名称匹配，不再根据数字 `5` 猜测机型。

设备卡片同时显示商品名和云端原始型号。尚未识别的内部标识显示“小米穿戴设备（型号待识别）”，保留原型号以便补充识别。

目录还提供 SPP SAR 协议版本提示：Redmi Watch 4、小米手环 8 Pro、Xiaomi Watch S1 Pro 使用版本 1；其他目录中的 SPP 设备使用版本 2。未知型号沿用版本 2 尝试连接。目录不是安装白名单，也不表示所有列出的设备都支持 SPP 或第三方应用。

旧缓存中的名称和 `is_supported` 标记不作为判断依据；载入时根据原始 `model` 重新计算名称、`is_verified` 和兼容说明。

## 数据来源

- [小米 Vela 概述：跨设备运行与多屏适配](https://iot.mi.com/vela/quickapp/en/guide/)
- [小米官方 fetch 接口支持明细](https://iot.mi.com/vela/quickapp/en/features/network/fetch.html#support-details)
- [OronBox 设备目录：型号别名与 SPP 版本](https://github.com/zxor-org/OronBox/blob/main/lib/src/device/core/xiaomi_wearable_catalog.dart)
- [get-mi-watchface 型号对照](https://github.com/aurysian-yan/get-mi-watchface#设备映射表)

部分第三方目录的网络能力标记与官方文档不同。本项目采用官方接口文档给出提示，实际能力仍以具体设备和固件实测为准。
