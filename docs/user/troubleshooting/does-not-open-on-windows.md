---
title: WordCraft does not open on Windows
description: Start WordCraft with a different graphics backend if it flashes and quits, or quits with "Parent device is lost" in the log.
order: 20
verified: 0.5.0
---

Start WordCraft with another graphics backend by setting `WGPU_BACKEND` to `vulkan` in PowerShell. This avoids two known graphics-driver problems.

## Symptoms

- WordCraft flashes in Task Manager and quits before its window appears. This was seen with AMD drivers (`atio6axx.dll`).
- WordCraft quits right after its window appears, and the log says "Parent device is lost".

## Fix

1. Open PowerShell in the folder that contains `wordcraft.exe`.
2. Run these commands:

        $env:WGPU_BACKEND = "vulkan"
        & .\wordcraft.exe
        Remove-Item Env:WGPU_BACKEND

   The last line restores the default for later launches.

If this works, use the same steps each time you start WordCraft. Setting `WGPU_BACKEND` to `gl` can bring the first crash back on affected systems.

## Why this happens

By default, WordCraft starts only DirectX 12 on Windows. Some graphics drivers crash when OpenGL is also created, and that is why the default leaves OpenGL out. For shaders, WordCraft uses FXC, the compiler built into Windows, and ignores any `dxcompiler.dll` another program put on your `PATH`. The variable `WGPU_DX12_COMPILER` (`fxc`, `dxc` or `auto`) overrides this. With `dxc`, matching `dxcompiler.dll` and `dxil.dll` files must sit next to `wordcraft.exe`.

If your PC is old and WordCraft feels slow, it may be using Microsoft's software renderer. When DirectX 12 finds no graphics card with a driver for it, WordCraft switches to OpenGL by itself. The log file records the choice on lines starting with `graphics:`. See [Get help and find the log file](get-help-and-find-the-log.md) for where the log is.

## Still stuck?

Ask in [Discord](https://discord.gg/artcraft) or open an issue in the [issue tracker](https://github.com/storytold/wordcraft/issues). Attach the log file.
