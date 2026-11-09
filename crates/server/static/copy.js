// The home page's copy button: copies the line to paste to an agent, from the element
// that shows it. The button starts hidden, so a browser without script shows no dead
// button. Where the Clipboard API refuses, as on plain HTTP, it selects the line and
// names the key that copies it.
const copyKey = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘C" : "Ctrl+C";
for (const button of document.querySelectorAll("button[data-copy]")) {
  const line = document.getElementById(button.dataset.copy);
  const label = button.textContent;
  // A screen reader doesn't announce a button's new label, so a status says it. It
  // goes after the paragraph, so a selection of the line leaves it out.
  const status = document.createElement("span");
  status.className = "visually-hidden";
  status.setAttribute("role", "status");
  button.closest("p").after(status);
  let reset;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(line.textContent);
      button.textContent = "Copied";
      status.textContent = "Copied the line to paste to your agent.";
    } catch {
      getSelection().selectAllChildren(line);
      button.textContent = `Press ${copyKey}`;
      status.textContent = `Selected the line to paste to your agent: press ${copyKey} to copy it.`;
    }
    clearTimeout(reset);
    reset = setTimeout(() => {
      button.textContent = label;
      status.textContent = "";
    }, 2000);
  });
  button.hidden = false;
}
