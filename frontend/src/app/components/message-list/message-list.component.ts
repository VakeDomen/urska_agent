import { Component, ElementRef, Input, OnChanges, SecurityContext, SimpleChanges, ViewChild } from '@angular/core'
import { CommonModule } from '@angular/common'
import { trigger, style, transition, animate, state } from '@angular/animations'
import { Message } from '../../models/message.model'
import { MarkdownModule, MarkdownService, SECURITY_CONTEXT } from 'ngx-markdown';

@Component({
  selector: 'message-list',
  standalone: true,
  imports: [CommonModule, MarkdownModule],
  templateUrl: './message-list.component.html',
  styleUrls: ['./message-list.component.css'],
  providers: [
    MarkdownService,
    { provide: SECURITY_CONTEXT, useValue: SecurityContext.HTML },
  ],
  animations: [
    trigger('fadeIn', [
      transition(':enter', [
        style({ opacity: 0 }),
        animate('200ms', style({ opacity: 1 }))
      ])
    ]),
    // Animation for the thinking box to collapse smoothly
    trigger('collapse', [
      state('false', style({ height: '0px', opacity: 0, margin: '0', padding: '0' })),
      state('true', style({ height: '*', opacity: 1 })),
      transition('true => false', animate('300ms ease-in-out')),
    ])
  ]
})
export class MessageListComponent implements OnChanges {

  @Input() messages: Message[] = []
  @Input() newToken: String | undefined;

  @ViewChild('scrollContainer') private scrollContainer!: ElementRef;
  @ViewChild('scrollContainerThink') private scrollContainerThink!: ElementRef;

  ngOnChanges(changes: SimpleChanges): void {
    // Append the new token to the content of the last message
    if (changes['newToken'] && this.messages.length > 0) {
      const token = changes['newToken'].currentValue;
      if (token) {
        // We assume the stream always targets the last message
        this.messages[this.messages.length - 1].content += token;
        this.scrollChatToBottom();
        this.scrollThinkToBottom();
      }
    }
  }

  private scrollChatToBottom(): void {
    try {
      if (this.scrollContainer) {
        const element = this.scrollContainer.nativeElement;
        element.scrollTop = element.scrollHeight;
      }
    } catch (err) {
      console.error('Could not scroll to bottom:', err);
    }
  }

  private scrollThinkToBottom(): void {
    try {
      if (this.scrollContainerThink) {
        const element = this.scrollContainerThink.nativeElement;
        element.scrollTop = element.scrollHeight;
      }
    } catch (err) {
      console.error('Could not scroll to bottom:', err);
    }
  }



  /**
   * Determines if a message is currently in the "thinking" phase.
   * This is true if the content starts with <think> but has not yet received </think>.
   */
  public isThinking(message: Message): boolean {
    const content = message.content;
    return content.startsWith('<think>') && !content.includes('</think>');
  }

  /**
   * Extracts the content for the thinking box.
   * This is the text between the <think> tag and the end of the current string.
   */
  public getThinkingContent(message: Message): string {
    return message.content.substring('<think>'.length);
  }

  /**
   * Determines if the final response for a message should be rendered.
   * This is true if the message never had a <think> tag, or if it has received the closing </think> tag.
   */
  public shouldRenderFinalContent(message: Message): boolean {
    const content = message.content;
    return !content.startsWith('<think>') || content.includes('</think>');
  }

  /**
   * Extracts the final, displayable content.
   * If there were <think> tags, it returns only the content that came *after* </think>.
   * Otherwise, it returns the full content.
   */
  public getFinalContent(message: Message): string {
    const content = message.content;
    const thinkEndTag = '</think>';
    const thinkEndIndex = content.indexOf(thinkEndTag);

    if (thinkEndIndex !== -1) {
      const raw = content.substring(thinkEndIndex + thinkEndTag.length);
      return this.processLinks(raw);
    }

    if (!content.startsWith('<think>')) {
      return this.processLinks(content);
    }

    return '';
  }


  /**
   * Replaces Markdown links with HTML <a> tags that open in a new tab.
   * Skips image links and preserves optional title text.
   * Also converts bare autolinks like <https://example.com>.
   */
  private processLinks(markdown: string): string {
    if (!markdown) return markdown;

    // Replace [text](url "title") and [text](url)
    // Ignore image syntax starting with "!["
    const mdLinkRegex = /(!)?\[(?<text>[^\]]+)\]\((?<url>\S+?)(?:\s+"(?<title>[^"]*)")?\)/g;

    const replaced = markdown.replace(mdLinkRegex, (match, bang, _text, _url, _title, offset, full) => {
      if (bang) return match; // leave images unchanged

      // Extract named groups safely
      const groups = (mdLinkRegex as any).lastMatch?.groups || (match as any).groups; // TS appeasement
      const text = groups?.text ?? _text;
      const url = groups?.url ?? _url;
      const title = groups?.title ?? _title;

      // Basic guard against javascript: and data: URLs
      const safeUrl = /^https?:\/\//i.test(url) || url.startsWith('/') ? url : '#';

      const titleAttr = title ? ` title="${this.escapeHtml(title)}"` : '';
      return `<a href="${this.escapeHtml(safeUrl)}"${titleAttr} target="_blank" rel="noopener noreferrer">${this.escapeHtml(text)}</a>`;
    });

    // Replace autolinks like <https://example.com>
    const autoLinkRegex = /<((?:https?:\/\/)[^>\s]+)>/gi;
    const replacedAuto = replaced.replace(autoLinkRegex, (_m, url) => {
      const safeUrl = url;
      const label = this.escapeHtml(url);
      return `<a href="${this.escapeHtml(safeUrl)}" target="_blank" rel="noopener noreferrer">${label}</a>`;
    });

    return replacedAuto;
  }

  private escapeHtml(str: string): string {
    return str
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }
}
