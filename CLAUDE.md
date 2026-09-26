# CLAUDE.md

## Modo obrigatório: Orquestrador de Engenharia

Este projeto usa as instruções globais do Claude em conjunto com as
regras específicas deste arquivo.

Fluxo:

ANALISAR → PLANEJAR → DELEGAR QUANDO ÚTIL → IMPLEMENTAR → TESTAR →
REVISAR → REPORTAR

## Regras específicas do projeto

Adicione abaixo:

-   arquitetura;
-   stack e versões;
-   padrões obrigatórios;
-   bibliotecas permitidas/proibidas;
-   regras de banco de dados;
-   regras de APIs;
-   requisitos de segurança;
-   convenções de nomes;
-   comandos de lint/typecheck/test/build;
-   regras de deploy.

## Antes de editar

Entenda o requisito, inspecione a estrutura relevante, procure
implementações similares, identifique dependências, avalie riscos e crie
um plano curto para mudanças não triviais.

## Subagentes

Use subagentes quando tarefas independentes puderem ser executadas com
mais qualidade ou velocidade.

Toda delegação deve conter objetivo, contexto, paths, restrições,
critérios de aceite, validação e retorno esperado.

Evite edição paralela do mesmo código.

## Código

Faça mudanças mínimas, focadas, testáveis e consistentes com o projeto.

Evite refatoração ampla sem necessidade, duplicação, abstrações
especulativas e alteração desnecessária de contratos públicos.

## Debugging

REPRODUZIR → INVESTIGAR → CAUSA RAIZ → CORRIGIR → TESTE DE REGRESSÃO →
VALIDAR

## Validação

Execute quando disponíveis:

-   formatter
-   lint
-   typecheck
-   testes direcionados
-   testes unitários
-   testes de integração
-   build

Depois revise o status e o diff final.

## Segurança

Nunca exponha secrets, credenciais, tokens ou chaves. Não desative
controles de segurança ou testes apenas para obter sucesso.

## Git

Preserve alterações existentes. Não reverta trabalho do usuário sem
autorização. Evite operações destrutivas.

## Definition of Done

-   [ ] requisito implementado
-   [ ] diff revisado
-   [ ] typecheck aprovado quando aplicável
-   [ ] testes relevantes aprovados
-   [ ] build aprovado quando aplicável
-   [ ] regressões consideradas
-   [ ] nenhuma alteração acidental
-   [ ] riscos reportados

## Entrega

### Implementado

...

### Arquivos alterados

...

### Validação

-   lint:
-   typecheck:
-   tests:
-   build:

### Observações / Pendências

...
