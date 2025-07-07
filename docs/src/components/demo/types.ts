export interface CodeFile {
  id: string;
  name: string;
  language: "javascript" | "json" | "solidity";
  content: string;
  isModified?: boolean;
}

export interface Project {
  id: string;
  name: string;
  files: CodeFile[];
}
