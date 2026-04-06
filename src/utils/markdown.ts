function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function applyInlineMarkdown(value: string): string {
  return value
    .replace(
      /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
      '<a href="$2" target="_blank" rel="noreferrer">$1</a>',
    )
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_]+)__/g, "<strong>$1</strong>")
    .replace(/(^|[^\*])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(/(^|[^_])_([^_\n]+)_/g, "$1<em>$2</em>");
}

export function markdownToHtml(markdown: string): string {
  if (!markdown.trim()) {
    return "";
  }

  const codeBlocks: string[] = [];
  let html = escapeHtml(markdown).replace(
    /```([\s\S]*?)```/g,
    (_match, code: string) => {
      const index = codeBlocks.push(
        `<pre><code>${code.trim().replace(/\n/g, "<br />")}</code></pre>`,
      );
      return `__CODE_BLOCK_${index - 1}__`;
    },
  );

  const lines = html.split(/\r?\n/);
  const blocks: string[] = [];
  let listItems: string[] = [];

  const flushList = () => {
    if (listItems.length > 0) {
      blocks.push(`<ul>${listItems.join("")}</ul>`);
      listItems = [];
    }
  };

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) {
      flushList();
      continue;
    }

    const headerMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (headerMatch) {
      flushList();
      const level = headerMatch[1].length;
      blocks.push(
        `<h${level}>${applyInlineMarkdown(headerMatch[2].trim())}</h${level}>`,
      );
      continue;
    }

    const listMatch = line.match(/^[-*]\s+(.+)$/);
    if (listMatch) {
      listItems.push(`<li>${applyInlineMarkdown(listMatch[1].trim())}</li>`);
      continue;
    }

    flushList();
    blocks.push(`<p>${applyInlineMarkdown(line)}</p>`);
  }

  flushList();

  return blocks.join("").replace(/__CODE_BLOCK_(\d+)__/g, (_match, index: string) => {
    return codeBlocks[Number(index)] ?? "";
  });
}
