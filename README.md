# SolHat User Interface (GTK4)

## Building - Fedora
Install [Rust](rust-lang.org), then execute the following the ensure the correct dependencies are present:
```bash
sudo dnf group install -y "Development Tools"
sudo dnf install -y gtk4-devel gtk4-devel-tools rpm-build 
```

## Building - Ubuntu
Install [Rust](rust-lang.org). Most version of Ubuntu, as of this writing, don't seem to support GTK4 yet, with the exception of `22.10` Kinetic Kudu. 
You will need to execute the following to ensure the correct dependencies are present: 
```bash
sudo apt-get update 
sudo apt-get install -y libgtk-4-dev
```

## Building - Windows
To build in Windows (natively, not in Windows Subsystem for Linux), install the latest versions of MS Visual Studio (Community edition is sufficient), and Rust. Then, for GTK4, follow the instructions outlined by the [The GTK Book](https://gtk-rs.org/gtk4-rs/stable/latest/book/installation_windows.html) and [gvsbuild](https://github.com/wingtk/gvsbuild#development-environment). When that is complete, you'll then need to run the following in PowerShell to add SVG support:
```powershell
cd C:\gtk-build
gvsbuild build librsvg
```
Each time `solhat-ui` is built, make sure to add GTK4 to the searchable paths with the following (from `gvsbuild`):
```powershell
$env:Path = "C:\gtk-build\gtk\x64\release\bin;" + $env:Path
$env:LIB = "C:\gtk-build\gtk\x64\release\lib;" + $env:LIB
$env:INCLUDE = "C:\gtk-build\gtk\x64\release\include;C:\gtk-build\gtk\x64\release\include\cairo;C:\gtk-build\gtk\x64\release\include\glib-2.0;C:\gtk-build\gtk\x64\release\include\gobject-introspection-1.0;C:\gtk-build\gtk\x64\release\lib\glib-2.0\include;" + $env:INCLUDE
```