import { useState, useCallback, useRef } from "react";

interface WikiSearchProps {
  query: string;
  onSearch: (query: string) => void;
}

export function WikiSearch({ query, onSearch }: WikiSearchProps) {
  const [value, setValue] = useState(query);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const v = e.target.value;
      setValue(v);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => onSearch(v), 300);
    },
    [onSearch],
  );

  return (
    <div className="wiki-search">
      <input
        type="text"
        className="wiki-search-input"
        placeholder="Search wiki..."
        value={value}
        onChange={handleChange}
      />
    </div>
  );
}
