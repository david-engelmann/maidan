### A copy button's name starts with the word it shows

- **Fixed:** The code block's Copy button and the catalog's Copy name button no longer put an `aria-label` over their visible words. Their names are what they show plus hidden context ("Copy code", "Copied code", "Copy name of this tool", "Copied tool name"), so after a copy the name still contains the "Copied" a voice-control user sees (WCAG 2.5.3).
