# Reading the Results and Saving a PDF

## The document data

After a scan, the area **Dokumentdaten** (*document data*) shows what was read
from the MRZ:

| Field | Meaning |
|---|---|
| Dokumenttyp | document type |
| Familienname | surname |
| Vornamen | given names |
| Dokumentnummer | document number |
| Staatsangehörigkeit | nationality |
| Geburtsdatum | date of birth |
| Geschlecht | sex |
| Gültig bis | date of expiry |
| Ausstellender Staat | issuing state |

The badge next to it tells you how sure the result is:

| Badge | Meaning |
|---|---|
| **MRZ geprüft** | *MRZ checked.* All check digits are correct. You can trust the data. |
| **MRZ ungeprüft** | *MRZ not checked.* Text was read, but the check digits do not match. Compare it with the document by eye. |
| **Keine MRZ** | *No MRZ.* Nothing was found. This is normal for the front of most ID cards. |

**Prüfziffer abweichend** (*check digit differs*) marks a single field whose
check digit is wrong.

Names have no check digit. Always compare names with the document.

## The pictures

Click **Vorderseite** (*front*) or **Rückseite** (*back*) and **Weißlicht**
(*white light*) or **Infrarot** (*infrared*) to switch between the four
pictures. If the document lay upside down, the program turns the picture the
right way up for you.

## Save as PDF

1. Select the document in the list.
2. Click **PDF**.
3. If no PDF folder is set (see chapter [Settings](#settings)), choose a folder
   and click **Speichern** (*save*). With a PDF folder set, the file is saved
   there straight away and the program shows where.

The file name is made up of the date, a running number and the time, for
example `2026-10-17--003--14-32-05.pdf`: the third PDF saved that day in that
folder, at 14:32:05. The number starts again at 001 every day and never
overwrites an existing file.

The PDF has four pages: front white, back white, front infrared, back infrared.
The PDF button only works when all four pictures exist; it shows
**PDF: 2/4 Bilder** (*2 of 4 pictures*) until then.

## The document list: right-click menu

Right-click a document in the list to open its menu:

| Entry | What it does |
|---|---|
| **Speichern unter …** (*save as*) | Saves the PDF to a place you choose, even if a PDF folder is set. Needs all four pictures. |
| **Löschen** (*delete*) | Removes the document, front and back, from the list. This cannot be undone. |

Right-click again, press **Esc** or click a document to close the menu.

## Important: save before you close

The program does **not** keep documents after you close it. Save every PDF you
need before closing the window.

Documents may contain personal data. Store the PDFs safely and delete them when
you no longer need them.
