export async function fetchStock(sku: string): Promise<number> {
  const res = await fetch(`/api/stock/${sku}`);
  const body = await res.json();
  return body.available;
}
