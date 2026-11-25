import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

/**
 * 版式红线（UI-AUDIT §6.1 防回归）：把「这轮审出来、下轮又长回来」的问题变成报错。
 * 这里只放**当前全站零违例**的条目——已经干净的东西没理由允许它长回去。
 * 裸状态色在阶段 2 已全部换成 --sem-*-text（预算归零），因此也升级为硬门禁。
 * className 既可能是字符串字面量也可能是模板串，两种都盖住。
 */
const banClass = (re, message) => ({
  selector: `:matches(Literal[value=${re}], TemplateElement[value.raw=${re}])`,
  message,
});

export default tseslint.config(
  { ignores: ["dist/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      // 唯一必须的红线：钩子不能出现在提前 return 之后（曾经整窗崩黑过一次）。
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
      "no-restricted-syntax": [
        "error",
        banClass(
          "/(^|\\s)text-\\[[1-9]px\\]/",
          "禁用 9px 及以下字号：正文最低 text-[11px]，徽标与单位最低 text-[10px]（字号阶梯见 docs/UI-ALIGNMENT.md）。",
        ),
        banClass(
          "/(^|\\s)outline-none/",
          "禁用 outline-none：它会吃掉全局 :focus-visible 焦点环，键盘用户就找不到焦点了。要改焦点样式请覆盖 --focus。",
        ),
        banClass(
          "/(^|\\s)(?:hover:|focus:)?text-(?:rose|red|emerald|amber)-[0-9]/",
          "禁用裸状态色文字：语义色一律走 var(--sem-ok-text) / --sem-danger-text / --sem-busy-text，否则浅色主题下对比度不达标（token 见 styles.css）。",
        ),
      ],
    },
  },
);
