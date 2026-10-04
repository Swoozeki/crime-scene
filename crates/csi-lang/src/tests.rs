use super::*;

fn fns(m: &FileMetrics) -> Vec<(&str, u32)> {
    m.functions.iter().map(|f| (f.name.as_str(), f.cc)).collect()
}

const COMPONENT: &str = r#"
import { Component, inject, input, output } from '@angular/core';

@Component({
  selector: 'app-cart',
  templateUrl: './cart.component.html',
  styleUrls: ['./cart.component.scss'],
})
export class CartComponent {
  items = input<Item[]>([]);
  checkout = output<void>();
  private store = inject(Store);

  constructor(private http: HttpClient, private router: Router) {}

  total(): number {
    let sum = 0;
    for (const i of this.items()) {
      if (i.qty > 0 && i.price) {
        sum += i.qty * i.price;
      } else if (i.free) {
        continue;
      }
    }
    return sum > 100 ? sum * 0.9 : sum;
  }

  ngOnInit() {
    this.store.select(x => x.cart).subscribe(cart => {
      if (cart) { this.load(cart); }
    });
  }

  handler = (e: Event) => { return e ?? null; };
}

export function helper(a: number) { return a || 1; }
"#;

#[test]
fn typescript_functions_and_cc() {
    let m = analyze("src/app/cart/cart.component.ts", COMPONENT.as_bytes(), 1 << 20).unwrap();
    assert_eq!(m.lang, Lang::TypeScript);
    let f = fns(&m);
    // for + if + && + else-if + ternary = 5 → cc 6
    assert!(f.contains(&("CartComponent.total", 6)), "{f:?}");
    // anonymous callbacks fold into ngOnInit: if → cc 2
    assert!(f.contains(&("CartComponent.ngOnInit", 2)), "{f:?}");
    assert!(f.contains(&("CartComponent.handler", 2)), "{f:?}");
    assert!(f.contains(&("helper", 2)), "{f:?}");
    assert!(f.contains(&("CartComponent.constructor", 1)), "{f:?}");
    let total = m.functions.iter().find(|f| f.name == "CartComponent.total").unwrap();
    assert_eq!(total.nesting, 2, "else-if must not add nesting");
    let a = m.summary.angular.as_ref().unwrap();
    assert_eq!(a.kind, "component");
    assert_eq!(a.deps, 3);
    assert_eq!(a.inputs, 1);
    assert_eq!(a.outputs, 1);
    assert_eq!(m.summary.template_url.as_deref(), Some("./cart.component.html"));
    assert_eq!(m.summary.style_urls, vec!["./cart.component.scss"]);
    assert_eq!(m.summary.health, 10.0);
}

#[test]
fn spec_callbacks_are_named_by_call() {
    let src = "describe('Cart', () => {\n  it('adds', () => { expect(1).toBe(1); });\n});\n";
    let m = analyze("cart.spec.ts", src.as_bytes(), 1 << 20).unwrap();
    assert_eq!(m.functions[0].name, "describe('Cart')");
}

const TEMPLATE: &str = r#"
<div class="cart">
  @if (items().length > 0) {
    @for (item of items(); track item.id) {
      <app-line [item]="item" />
      @if (item.discount && item.vip) { <span>VIP</span> }
    }
  } @else if (loading) {
    <spinner/>
  } @else {
    <p>Empty</p>
  }
  <ul *ngIf="legacy">
    <li *ngFor="let x of xs">{{ x.name ?? 'n/a' }}</li>
  </ul>
</div>
"#;

#[test]
fn angular_template_blocks() {
    let m = analyze("cart.component.html", TEMPLATE.as_bytes(), 1 << 20).unwrap();
    assert_eq!(m.lang, Lang::Template);
    assert!(m.functions.len() >= 2, "{:?}", fns(&m));
    assert!(m.functions[0].name.starts_with("@if"), "{:?}", fns(&m));
    assert!(m.complexity >= 7.0, "complexity {}", m.complexity);
    assert!(m.summary.max_nesting >= 2);
}

const PHP: &str = r#"<?php
namespace App\Services;

class OrderService
{
    public function __construct(private Repo $repo) {}

    public function total(array $items): float
    {
        $sum = 0;
        foreach ($items as $i) {
            if ($i->qty > 0 && $i->price) {
                $sum += $i->qty * $i->price;
            } elseif ($i->free) {
                continue;
            }
        }
        $fn = function ($x) { return $x ?? 0; };
        return match (true) { $sum > 100 => $sum * 0.9, default => $sum };
    }
}

function helper($a) { return $a or 1; }
"#;

#[test]
fn php_methods() {
    let m = analyze("app/Services/OrderService.php", PHP.as_bytes(), 1 << 20).unwrap();
    let f = fns(&m);
    // foreach + if + && + elseif + ?? (closure folded) + match arm = 6 → cc 7
    assert!(f.contains(&("OrderService::total", 7)), "{f:?}");
    assert!(f.contains(&("helper", 2)), "{f:?}");
    assert_eq!(m.summary.methods, 2);
}

#[test]
fn scss_rules() {
    let src = ".cart {\n  .line {\n    &:hover { color: red; }\n  }\n}\n@media (max-width: 600px) {\n  .cart { display: none; }\n}\n";
    let m = analyze("cart.component.scss", src.as_bytes(), 1 << 20).unwrap();
    assert_eq!(m.functions[0].name, ".cart");
    assert_eq!(m.functions[0].cc, 3);
    assert_eq!(m.functions[0].nesting, 2);
}

#[test]
fn health_penalizes_brain_functions() {
    let mut body = String::from("function big(x: number) {\n");
    for i in 0..25 {
        body.push_str(&format!("  if (x === {i}) {{ return {i}; }}\n"));
    }
    body.push_str("  return 0;\n}\n");
    let m = analyze("big.ts", body.as_bytes(), 1 << 20).unwrap();
    assert_eq!(m.max_cc, 26);
    assert!(m.summary.health < 9.0);
    assert!(m.summary.health_reasons[0].contains("`big` cc 26"));
}

#[test]
fn fallback_and_binary() {
    let m = analyze("deploy.sh", b"if x; then\n    echo a\n        echo b\nfi\n", 1 << 20).unwrap();
    assert_eq!(m.lang, Lang::Other);
    assert_eq!(m.complexity, 3.0);
    assert!(analyze("x.bin", &[0, 1, 2], 1 << 20).is_none());
}

#[test]
fn generated_detection() {
    let m = analyze("api.ts", b"// This file was generated by openapi\nexport const a = 1;\n", 1 << 20).unwrap();
    assert!(m.generated);
}

#[test]
fn test_paths() {
    assert!(is_test_path("src/app/cart.component.spec.ts"));
    assert!(is_test_path("tests/Unit/OrderTest.php"));
    assert!(!is_test_path("src/app/cart.component.ts"));
    assert!(is_test_path("modules/store/spec/marbles.ts"));
    assert!(is_test_path("spec/helpers.ts"));
    assert!(is_test_path("src/api/__mocks__/client.ts"));
    assert!(is_test_path("src/app/cart.mock.ts"));
    assert!(is_test_path("tests/fixtures/orders.json"));
    assert!(!is_test_path("modules/store/testing/src/testing.ts")); // a shipped testing API
}

#[test]
fn function_lookup_prefers_innermost() {
    let fs = vec![
        Function { name: "outer".into(), start: 1, end: 20, cc: 1, nesting: 0, loc: 20 },
        Function { name: "inner".into(), start: 5, end: 8, cc: 1, nesting: 0, loc: 4 },
    ];
    assert_eq!(function_at(&fs, 6).unwrap().name, "inner");
    assert_eq!(function_at(&fs, 10).unwrap().name, "outer");
    assert!(function_at(&fs, 30).is_none());
}
