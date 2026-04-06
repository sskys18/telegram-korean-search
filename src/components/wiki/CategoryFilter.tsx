import type { WikiCategory } from "../../types";

interface CategoryFilterProps {
  categories: WikiCategory[];
  activeCategoryId?: number;
  onChange: (categoryId?: number) => void;
}

export function CategoryFilter({
  categories,
  activeCategoryId,
  onChange,
}: CategoryFilterProps) {
  return (
    <div className="category-filter" aria-label="Wiki categories">
      <button
        type="button"
        className={
          activeCategoryId == null
            ? "category-pill category-pill-active"
            : "category-pill"
        }
        onClick={() => onChange(undefined)}
      >
        All
      </button>
      {categories.map((category) => (
        <button
          key={category.category_id}
          type="button"
          className={
            activeCategoryId === category.category_id
              ? "category-pill category-pill-active"
              : "category-pill"
          }
          onClick={() => onChange(category.category_id)}
        >
          {category.name_ko || category.name}
        </button>
      ))}
    </div>
  );
}
