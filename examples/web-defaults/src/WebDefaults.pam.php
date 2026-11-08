<?php

declare(strict_types=1);

namespace App;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Charts\ChartTheme;
use Pam\Desktop\Charts\LegendStyle;
use Pam\Desktop\Component;

final class WebDefaults extends Component
{
    /** @var list<Order> */
    #[State] public array $orders = [];
    public ChartTheme $chartTheme;

    public function __construct()
    {
        // Chart.js legend metrics: 12px labels, 6px dots, 20px between items.
        $this->chartTheme = ChartTheme::dark()->legendStyle(new LegendStyle(fontSize: 12, markerRadius: 3, itemGap: 20));
        for ($i = 1; $i <= 3; $i++) {
            $this->add();
        }
    }

    public function add(): void
    {
        $id = count($this->orders) + 1;
        $this->orders = [...$this->orders, new Order($id, $id % 2 === 0 ? OrderStatus::Paid : OrderStatus::Open, 980 * $id)];
    }

    /** @return list<int> */
    public function totals(): array
    {
        return array_map(static fn (Order $order): int => $order->cents, $this->orders);
    }

}
?>
<template>
    <Column class="page">
        <Text class="title">Web defaults</Text>
        <Text class="mono">ui-monospace, Menlo, Consolas, monospace</Text>
        <Text class="sans">system-ui, -apple-system, sans-serif</Text>
        <Button on:press="add">Add order</Button>
        <Row gap="16" flexGrow="1" minHeight="0">
            <!-- No height: the card follows its rows, up to the space left. -->
            <Column class="card">
                <Text class="heading">{{ count($orders) }} orders</Text>
                <VirtualizedList estimatedRowHeight="44" class="orders">
                    <Row p-for="$order in $orders" p-key="$order->id" class="order">
                        <Text flexGrow="1">{{ '#'.$order->id.' · '.$order->status->label() }}</Text>
                        <Text class="mono">{{ $order->total() }}</Text>
                    </Row>
                </VirtualizedList>
            </Column>
            <Column gap="12">
                <!-- The line axis fits its data; the bars begin at zero. -->
                <LineChart width="300" height="130" grid="6" :values="totals()" :theme="$chartTheme" />
                <BarChart width="300" height="130" :values="[980, 1500, 2040]" :theme="$chartTheme" />
                <DonutChart size="110" :slices="[['label' => 'Pix', 'value' => 70], ['label' => 'Card', 'value' => 30]]" :legend="\Pam\Desktop\Charts\LegendPosition::Bottom" :legendInline="true" :legendValues="false" :theme="$chartTheme" />
            </Column>
        </Row>
    </Column>
</template>
<style scoped>
    .page { flex-grow: 1; padding: 24px; gap: 12px; background-color: #0C1117; }
    .title { font-family: system-ui, sans-serif; font-size: 24px; font-weight: 700; color: #F4F7FA; }
    .sans { font-family: system-ui, -apple-system, sans-serif; color: #A0ADBB; }
    .mono { font-family: ui-monospace, Menlo, Consolas, monospace; color: #A0ADBB; }
    .card { width: 340px; align-self: flex-start; max-height: 100%; min-height: 0; padding: 12px; gap: 8px; border-radius: 12px; background-color: #19222C; }
    .heading { color: #F4F7FA; font-weight: 600; }
    .orders { scrollbar-gutter: stable; scrollbar-width: thin; scrollbar-color: rgba(255, 255, 255, 0.08) transparent; }
    .order { height: 44px; align-items: center; gap: 8px; color: #F4F7FA; }
</style>
