# Reading the Results and Saving a PDF

## The document data

After a scan, the area **Document data** shows what was read
from the MRZ:

| Field | Meaning |
|----|-----|
| Document type | passport, ID card or other travel document |
| Surname | family name |
| Given names | first names |
| Document number | number of the card or passport |
| Nationality | nationality of the holder |
| Date of birth | date of birth |
| Sex | sex as written in the MRZ |
| Valid until | date of expiry |
| Issuing state | country that issued the document |

The badge next to it tells you how sure the result is:

| Badge | Meaning |
|----|--------|
| **MRZ verified** | All check digits are correct. You can trust the data. |
| **MRZ unverified** | Text was read, but the check digits do not match. Compare it with the document by eye. |
| **No MRZ** | Nothing was found. This is normal for the front of most ID cards. |

**Check digit mismatch** marks a single field whose
check digit is wrong.

Names have no check digit. Always compare names with the document.

## The pictures

Click **Front** or **Back** and **White** (white light) or **IR** (infrared)
to switch between the four
pictures. If the document lay upside down, the program turns the picture the
right way up for you.

## Save as PDF

1. Select the document in the list.
2. Click **PDF**.
3. If no PDF folder is set (see chapter [Settings](#settings)), choose a folder
   and click **Save**. With a PDF folder set, the file is saved
   there straight away and the program shows where.

The file name is made up of the date, a running number and the time, for
example `2026-10-17--003--14-32-05.pdf`: the third PDF saved that day in that
folder, at 14:32:05. The number starts again at 001 every day and never
overwrites an existing file.

The PDF has four pages: front white, back white, front infrared, back infrared.
The PDF button only works when all four pictures exist; it shows
**PDF: 2/4 images** until then.

## The document list: right-click menu

Right-click a document in the list to open its menu:

| Entry | What it does |
|----|--------|
| **Save as …** | Saves the PDF to a place you choose, even if a PDF folder is set. Needs all four pictures. |
| **Delete** | Removes the document, front and back, from the list. This cannot be undone. |

Right-click again, press **Esc** or click a document to close the menu.

## Important: save before you close

The program does *not* keep documents after you close it. Save every PDF you
need before closing the window.

Documents may contain personal data. Store the PDFs safely and delete them when
you no longer need them.
