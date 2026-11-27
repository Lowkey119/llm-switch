/** 这个项目没有 clsx，就地拼 class（全站共用，别处不再各写一遍）。 */
export const cx = (...parts: Array<string | false | null>) =>
  parts.filter(Boolean).join(" ");
