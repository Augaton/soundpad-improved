# Global hotkey helper, driven by main.js over stdin/stdout.
# Electron's globalShortcut cannot register some keys (Pause/Break among them), so the
# hotkeys go straight through Win32 RegisterHotKey, the way Soundpad does it.
#
# stdin : "SET <seq> <id>:<mods>:<vk> <id>:<mods>:<vk> ..."  (replaces every registration)
# stdout: "READY" once, "DONE <seq> <failed ids, comma-separated>" per SET, "HIT <id>" per key press.
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

public static class SpiHotkeys {
  [StructLayout(LayoutKind.Sequential)]
  struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam; public uint time; public int x; public int y; }

  [DllImport("user32.dll")] static extern bool RegisterHotKey(IntPtr hWnd, int id, uint mods, uint vk);
  [DllImport("user32.dll")] static extern bool UnregisterHotKey(IntPtr hWnd, int id);
  [DllImport("user32.dll")] static extern int GetMessage(out MSG msg, IntPtr hWnd, uint min, uint max);
  [DllImport("user32.dll")] static extern bool PostThreadMessage(uint threadId, uint msg, IntPtr w, IntPtr l);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();

  const uint WM_QUIT = 0x0012, WM_HOTKEY = 0x0312, WM_COMMAND_LINE = 0x8001, MOD_NOREPEAT = 0x4000;

  public static void Run() {
    uint tid = GetCurrentThreadId();
    var lines = new ConcurrentQueue<string>();
    var registered = new List<int>();
    var reader = new Thread(() => {
      string line;
      while ((line = Console.ReadLine()) != null) {
        lines.Enqueue(line);
        PostThreadMessage(tid, WM_COMMAND_LINE, IntPtr.Zero, IntPtr.Zero);
      }
      PostThreadMessage(tid, WM_QUIT, IntPtr.Zero, IntPtr.Zero);  // parent closed stdin
    });
    reader.IsBackground = true;
    reader.Start();
    Console.WriteLine("READY");

    MSG msg;
    while (GetMessage(out msg, IntPtr.Zero, 0, 0) > 0) {
      if (msg.message == WM_HOTKEY) {
        Console.WriteLine("HIT " + msg.wParam.ToInt32());
      } else if (msg.message == WM_COMMAND_LINE) {
        string line;
        while (lines.TryDequeue(out line)) {
          var parts = line.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
          if (parts.Length < 2 || parts[0] != "SET") continue;
          foreach (var id in registered) UnregisterHotKey(IntPtr.Zero, id);
          registered.Clear();
          var failed = new List<string>();
          for (int i = 2; i < parts.Length; i++) {
            var f = parts[i].Split(':');
            int id; uint mods, vk;
            if (f.Length == 3 && int.TryParse(f[0], out id) && uint.TryParse(f[1], out mods) && uint.TryParse(f[2], out vk)
                && RegisterHotKey(IntPtr.Zero, id, mods | MOD_NOREPEAT, vk)) {
              registered.Add(id);
            } else {
              failed.Add(f[0]);
            }
          }
          Console.WriteLine("DONE " + parts[1] + " " + string.Join(",", failed));
        }
      }
    }
  }
}
'@
[SpiHotkeys]::Run()
