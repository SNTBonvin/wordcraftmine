---
title: WordCraft does not open on Windows
description: Start WordCraft with a different graphics backend if it flashes and quits, or quits with "Parent device is lost" in the log.
order: 20
verified: 0.5.0
---

Start WordCraft with another graphics backend by setting `WGPU_BACKEND` to `vulkan` in PowerShell. This avoids two known graphics-driver problems: a missing shader compiler file (`dxcompiler.dll`) on DirectX 12, and an AMD driver crash in OpenGL. On Intel graphics, use `gl` instead.

## Symptoms

- WordCraft quits before its window appears.
- WordCraft quits right after its window appears, and the log says "Parent device is lost".

## Fix

1. Open PowerShell in the folder that contains `wordcraft.exe`.
2. Run these commands:

        $env:WGPU_BACKEND = "vulkan"
        & .\wordcraft.exe
        Remove-Item Env:WGPU_BACKEND

   The last line restores the default for later launches.

**Intel graphics (UHD, Iris):** use `gl` in place of `vulkan` in the first line. Intel's Vulkan driver (`igvk64.dll`) can crash WordCraft at start ([issue 170](https://github.com/storytold/wordcraft/issues/170)).

If this works, use the same steps each time you start WordCraft. On AMD graphics do not use `gl`: it can bring back the `atio6axx.dll` crash.

## Why this happens

WordCraft 0.5.0 compiles its DirectX 12 shaders with DXC but does not ship `dxcompiler.dll` and `dxil.dll`, so on some PCs the graphics device is lost before the first frame and the log ends with `Parent device is lost` ([issue 603](https://github.com/storytold/wordcraft/issues/603)). A fix that uses FXC, the compiler built into Windows, was merged after 0.5.0 and is not in a release yet. The default leaves OpenGL out because creating it can crash AMD's `atio6axx.dll` before the window appears.

## Slow on an older PC

If your PC is old and WordCraft feels slow, it may be using Microsoft's software renderer. When DirectX 12 finds no graphics card with a driver for it, WordCraft switches to OpenGL by itself. The log file records the choice on lines starting with `graphics:`. See [Get help and find the log file](get-help-and-find-the-log.md) for where the log is.

## Still stuck?

Ask in [Discord](https://discord.gg/artcraft) or open an issue in the [issue tracker](https://github.com/storytold/wordcraft/issues). Attach the log file.
