export interface Message {
  role: 'user' | 'assistant'
  content: string
  timestamp: Date
  state: string | undefined
  error: string | undefined
}

export interface CountedToken {
  value: string,
  seq: number,
}

export interface CountedError {
  value: string,
  seq: number,
}