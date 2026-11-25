// The home page's copy button: copies the line to paste to an agent, from the element
// that shows it. The line is a slip with the button at its end, and a click anywhere
// on the slip presses the button. The button starts hidden, so a browser without
// script shows the line alone, with no dead button. Where the Clipboard API refuses,
// as on plain HTTP, it selects the line and names the key that copies it.
const copyKey = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘C" : "Ctrl+C";
for (const button of document.querySelectorAll("button[data-copy]")) {
  const line = document.getElementById(button.dataset.copy);
  const slip = button.closest("p");
  const label = button.querySelector("span");
  const idle = label.textContent;
  // A screen reader doesn't announce a button's new label, so a status says it. It
  // goes after the paragraph, so a selection of the line leaves it out.
  const status = document.createElement("span");
  status.className = "visually-hidden";
  status.setAttribute("role", "status");
  slip.after(status);
  // Struck across the slip on a copy, for the eye only: the status says it aloud.
  const stamp = document.createElement("span");
  stamp.className = "stamp copied";
  stamp.textContent = "Copied";
  stamp.setAttribute("aria-hidden", "true");
  stamp.hidden = true;
  slip.append(stamp);
  let reset;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(line.textContent);
      label.textContent = "Copied";
      status.textContent = "Copied the line to paste to your agent.";
      stamp.hidden = false;
    } catch {
      getSelection().selectAllChildren(line);
      // No longer than "Copied": a wider button would rewrap the line just selected.
      label.textContent = copyKey;
      status.textContent = `Selected the line to paste to your agent: press ${copyKey} to copy it.`;
    }
    clearTimeout(reset);
    reset = setTimeout(() => {
      label.textContent = idle;
      status.textContent = "";
      stamp.hidden = true;
    }, 2000);
  };
  slip.addEventListener("click", (event) => {
    // A drag that selects part of the line ends in a click too, and keeps what it selected.
    if (button.contains(event.target) || getSelection().isCollapsed) copy();
  });
  slip.classList.add("live");
  button.hidden = false;
}
