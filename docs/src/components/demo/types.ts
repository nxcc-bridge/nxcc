export interface CodeFile {
  id: string;
  name: string;
  language: 'javascript' | 'json';
  content: string;
}

export interface Project {
  id: string;
  name: string;
  files: CodeFile[];
}
