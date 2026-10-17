# Installation

## Step 1: Install Tesseract

Passport Scanner uses Tesseract to read text.

1. Download the Windows installer of Tesseract OCR from
   <https://github.com/UB-Mannheim/tesseract/wiki>.
2. Run it and keep the suggested folder.

Passport Scanner finds Tesseract in its usual folder by itself. If you put it
somewhere else, set the environment variable `TESSERACT` to the full path of
`tesseract.exe`.

## Step 2: Unpack Passport Scanner

1. Unpack the downloaded ZIP file into a folder of your choice. This guide
   uses `C:\Programs\Passport Scanner` as the example.
2. Keep all files together. The `ui` folder must stay next to the programs.

There is no setup program. To remove Passport Scanner, delete the folder.

## Step 3: Connect the scanner

Plug the scanner into a USB port. Windows sets it up by itself; no extra driver
is needed.

The scanner works best with the calibration that came from the factory. If the
manufacturer's software was ever installed on this PC, Passport Scanner uses
its calibration files automatically. If not, you can measure your own; see
chapter [Calibration](#calibration).

## Step 4: Start the program

Double-click `passport-scanner-desktop.exe`.

The window opens. The status at the top shows **Bereit** (*ready*) when the
scanner is connected and **Nicht verbunden** (*not connected*) when it is not.

Only one program can use the scanner at a time. Close other scanner programs
first.
