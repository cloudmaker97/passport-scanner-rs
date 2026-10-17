# Introduction

Passport Scanner reads ID cards and passports with a Plustek SecureScan
scanner. Put a document on the glass, press a button, and you get:

- a photo of the front and the back, in white light and in infrared light,
- the data from the machine-readable zone (the MRZ, the two or three lines of
  `<<<` text at the bottom of a passport or ID card),
- one PDF file with all four pictures.

Everything happens on your own computer. Nothing is uploaded anywhere.

## What you need

- A Windows PC.
- A Plustek SecureScan scanner on a USB port. The program is tested with the
  X-Mini 2000. Other models of the same family may work but are not tested.
- Tesseract OCR, a free program that reads the text. See chapter
  [Installation](#installation).

## Words used in this guide

| Word | Meaning |
|---|---|
| Document | One card or passport, with its front and back. |
| Side | The front (*Vorderseite*) or the back (*Rückseite*). |
| White light | A normal colour picture (*Weißlicht*). |
| Infrared | A grey picture taken with infrared light (*Infrarot*). Security features look different in it. |
| MRZ | Machine-readable zone. The lines of text with `<` characters. |
| Calibration | A measurement that makes the picture evenly bright from edge to edge. |

The program window is in German. This guide gives the German button name
first, in **bold**, and the English meaning after it.

## Not a Plustek product

This program is not made, sold, endorsed or supported by Plustek. Do not ask
Plustek for help with it. The names *Plustek* and *SecureScan* appear only to
say which scanner this program works with.
