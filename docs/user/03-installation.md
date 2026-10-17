# Installation

## Step 1: Install Tesseract

Passport Scanner uses Tesseract to read text.

1. Download the Windows installer of Tesseract OCR from
   <https://github.com/UB-Mannheim/tesseract/wiki>.
2. Run it and keep the suggested folder.

Passport Scanner finds Tesseract in its usual folder by itself. If you put it
somewhere else, set the environment variable `TESSERACT` to the full path of
`tesseract.exe`.

## Step 2: Download and unpack Passport Scanner

1. Open the releases page of the project,
   <https://github.com/cloudmaker97/passport-scanner-rs/releases>, and download
   the file for your system from the newest release:

   | System | File |
   |----|------|
   | Windows | `passport-scanner-windows-x64.zip` |
   | macOS (Apple silicon) | `passport-scanner-macos-arm64.zip` |
   | Linux | `passport-scanner-linux-x64.zip` |

2. Unpack the ZIP file into a folder of your choice. This guide uses
   `C:\Programs\Passport Scanner` as the example. The folder must be one you
   may write to, so the program can update itself.
3. Keep all files together. The `www` folder must stay next to the programs.
   This guide is in the folder too, as `Passport-Scanner-User-Guide.pdf`.

There is no setup program. To remove Passport Scanner, delete the folder.
Newer versions install themselves from within the program; see
[Updates](#updates).

## Step 3: Connect the scanner

Plug the scanner into a USB port. Windows sets it up by itself; no extra driver
is needed.

The scanner works best with the calibration that came from the factory. If the
manufacturer's software was ever installed on this PC, Passport Scanner uses
its calibration files automatically. If not, you can measure your own; see
chapter [Calibration](#calibration).

## Step 4: Start the program

Double-click `passport-scanner-desktop.exe`.

The window opens. The status at the top shows **Ready** when the scanner is
connected and **Not connected** when it is not.

Only one program can use the scanner at a time. Close other scanner programs
first.
