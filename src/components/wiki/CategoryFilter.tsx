import type { WikiCategory } from "../../types";

interface CategoryFilterProps {
  categories: WikiCategory[];
  selected: number | undefined;
  onSelect: (id: number | undefined) => void;
}

export function CategoryFilter({ categories, selected, onSelect }: CategoryFilterProps) {
  return (
    <div className="category-filter">
      <button
        className={`category-pill ${selected === undefined ? "category-active" : ""}`}
        onClick={() => onSelect(undefined)}
      >
        All
      </button>
      {categories.map((cat) => (
        <button
          key={cat.category_id}
          className={`category-pill ${selected === cat.category_id ? "category-active" : ""}`}
          onClick={() => onSelect(cat.category_id)}
        >
          {cat.name}
        </button>
      ))}
    </div>
  );
}
